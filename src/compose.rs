use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::{ComposeFileValue, DevContainerConfig, MountValue};
use crate::error::{BondarError, Result};

pub fn compose_files_args_for_build(
    config: &DevContainerConfig,
    config_path: &Path,
    workspace_folder: &Path,
) -> Result<Vec<String>> {
    compose_files_args(config, config_path, workspace_folder)
}

fn compose_files_args(
    config: &DevContainerConfig,
    config_path: &Path,
    workspace_folder: &Path,
) -> Result<Vec<String>> {
    let compose_val = config
        .docker_compose_file
        .as_ref()
        .ok_or_else(|| BondarError::Config("No dockerComposeFile".to_string()))?;
    let config_dir = config_path.parent().unwrap_or(Path::new("."));
    let files: Vec<String> = match compose_val {
        ComposeFileValue::Single(s) => vec![s.clone()],
        ComposeFileValue::Multiple(v) => v.clone(),
    };
    let mut args = Vec::new();
    for f in files {
        // Variable expansion is relative to the workspace root
        // (e.g. ${localWorkspaceFolder}), while path resolution is
        // relative to the devcontainer.json directory.
        let expanded = crate::docker::expand_vars_for_host(&f, workspace_folder);
        let path = config_dir.join(&expanded);
        if !path.exists() {
            eprintln!("Warning: compose file not found: {}", path.display());
        }
        let path_str = path.to_string_lossy().to_string();
        args.push("-f".to_string());
        args.push(path_str);
    }
    Ok(args)
}

fn mount_string_to_compose_volume(mount: &str) -> Option<String> {
    let mut source = None;
    let mut target = None;
    let mut readonly = false;
    let mut is_tmpfs = false;
    let mut is_bind = false;
    for part in mount.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((key, value)) = part.split_once('=') {
            match key {
                "type" => {
                    is_tmpfs = value == "tmpfs";
                    is_bind = value == "bind";
                }
                "source" | "src" => source = Some(value),
                "target" | "dst" | "destination" => target = Some(value),
                "readonly" | "ro" => readonly = value == "true" || value == "1",
                _ => {}
            }
        } else {
            match part {
                "readonly" | "ro" => readonly = true,
                _ => {}
            }
        }
    }
    // tmpfs mounts have no short-syntax equivalent in compose
    if is_tmpfs {
        return None;
    }
    let source = source.unwrap_or_default();
    // A relative bind source (not ./ or /) becomes a named volume in the
    // compose short syntax - warn about the behavioral difference
    if is_bind && !source.is_empty() && !source.starts_with('/') && !source.starts_with("./") {
        eprintln!(
            "Warning: bind mount source '{source}' is relative; compose short syntax will treat it as a named volume"
        );
    }
    let target = target?;
    let mut vol = String::new();
    if !source.is_empty() {
        vol.push_str(source);
        vol.push(':');
    }
    vol.push_str(target);
    if readonly {
        vol.push_str(":ro");
    }
    Some(vol)
}

fn write_compose_override(config: &DevContainerConfig, workspace_folder: &Path) -> Result<PathBuf> {
    let service = config
        .service
        .as_deref()
        .ok_or_else(|| BondarError::Config("No service specified".to_string()))?;
    let container_target = config
        .workspace_folder
        .clone()
        .unwrap_or_else(|| "/".to_string());

    let mut yaml = String::from("services:\n");
    yaml.push_str(&format!("  {}:\n", escape_yaml_key(service)));

    let mut ports: Vec<String> = Vec::new();
    for port in &config.forward_ports {
        let port_str = match port {
            crate::config::ForwardPort::Number(n) => n.to_string(),
            crate::config::ForwardPort::Text(s) => s.clone(),
        };
        if let Some(publish) = crate::docker::publish_port_arg(&port_str) {
            if crate::docker::is_port_ignored(config, &port_str) {
                println!(
                    "Skipping forwardPorts '{port_str}' in compose override (onAutoForward: ignore)"
                );
                continue;
            }
            let entry =
                if crate::docker::is_udp_port(config, &port_str) && !publish.ends_with("/udp") {
                    format!("\"{publish}/udp\"")
                } else {
                    format!("\"{publish}\"")
                };
            if !ports.contains(&entry) {
                ports.push(entry);
            }
        } else {
            eprintln!(
                "Warning: forwardPorts '{port_str}' references a service host, cannot publish in compose override"
            );
        }
    }
    if let Some(app_port) = &config.app_port {
        let app_ports: Vec<String> = match app_port {
            crate::config::AppPortValue::Single(p) => vec![port_value_to_string(p)],
            crate::config::AppPortValue::Multiple(v) => {
                v.iter().map(port_value_to_string).collect()
            }
        };
        for p in app_ports {
            if crate::docker::is_port_ignored(config, &p) {
                println!("Skipping appPort '{p}' in compose override (onAutoForward: ignore)");
                continue;
            }
            if let Some(publish) = crate::docker::publish_port_arg(&p) {
                let entry = if crate::docker::is_udp_port(config, &p) && !publish.ends_with("/udp")
                {
                    format!("\"{publish}/udp\"")
                } else {
                    format!("\"{publish}\"")
                };
                if !ports.contains(&entry) {
                    ports.push(entry);
                }
            } else {
                eprintln!(
                    "Warning: appPort '{p}' references a service host, cannot publish in compose override"
                );
            }
        }
    }

    let mut volumes: Vec<String> = Vec::new();
    for m in &config.mounts {
        match m {
            MountValue::String(s) => {
                let expanded = crate::docker::expand_vars_for_host_with_target(
                    s,
                    workspace_folder,
                    &container_target,
                );
                if let Some(vol) = mount_string_to_compose_volume(&expanded) {
                    if volumes.contains(&vol) {
                        eprintln!(
                            "Warning: mount '{vol}' is declared more than once; keeping the first entry"
                        );
                    } else {
                        volumes.push(vol);
                    }
                }
            }
            MountValue::Object(obj) => {
                // tmpfs mounts have no short-syntax equivalent in compose
                if obj.mount_type.as_deref() == Some("tmpfs") {
                    eprintln!(
                        "Warning: tmpfs mount is skipped in compose override (no short-syntax equivalent)"
                    );
                    continue;
                }
                if let Some(target) = &obj.target {
                    let mut vol = String::new();
                    if let Some(source) = &obj.source {
                        let expanded = crate::docker::expand_vars_for_host_with_target(
                            source,
                            workspace_folder,
                            &container_target,
                        );
                        vol.push_str(&expanded);
                        vol.push(':');
                    }
                    let expanded_target = crate::docker::expand_vars_for_host_with_target(
                        target,
                        workspace_folder,
                        &container_target,
                    );
                    vol.push_str(&expanded_target);
                    if obj.readonly.unwrap_or(false) {
                        vol.push_str(":ro");
                    }
                    if volumes.contains(&vol) {
                        eprintln!(
                            "Warning: mount '{vol}' is declared more than once; keeping the first entry"
                        );
                    } else {
                        volumes.push(vol);
                    }
                }
            }
        }
    }

    let mut wrote_any = false;
    // Build env lines first so an empty `environment:` key is never emitted
    // (e.g. when secrets contain only file-path entries that cannot be resolved).
    let mut env_lines: Vec<(String, String)> = Vec::new();
    // Resolve in two passes so ${containerEnv:KEY} works regardless of map order
    let mut expanded_env: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for (k, v) in &config.container_env {
        let expanded =
            crate::docker::expand_vars_for_host_with_target(v, workspace_folder, &container_target);
        expanded_env.insert(k.clone(), expanded);
    }
    for (k, v) in &config.container_env {
        let from_map = crate::docker::expand_container_env_from_map(v, &expanded_env, Some(k));
        let resolved = crate::docker::expand_vars_for_host_with_target(
            &from_map,
            workspace_folder,
            &container_target,
        );
        env_lines.push((k.clone(), resolved));
    }
    for (k, v) in crate::docker::resolve_secrets(config) {
        if env_lines.iter().any(|(ek, _)| ek == &k) {
            eprintln!(
                "Warning: secret key '{k}' conflicts with an existing environment entry and will override it"
            );
        }
        env_lines.push((k, v));
    }
    // Deterministic output: sort keys so the generated YAML is stable
    env_lines.sort_by(|a, b| a.0.cmp(&b.0));
    if !env_lines.is_empty() {
        wrote_any = true;
        yaml.push_str("    environment:\n");
        for (k, v) in &env_lines {
            yaml.push_str(&format!(
                "      {}: \"{}\"\n",
                escape_yaml_key(k),
                escape_yaml_value(v)
            ));
        }
    }
    if !ports.is_empty() {
        wrote_any = true;
        yaml.push_str("    ports:\n");
        for p in &ports {
            yaml.push_str(&format!("      - {p}\n"));
        }
    }
    if !volumes.is_empty() {
        wrote_any = true;
        yaml.push_str("    volumes:\n");
        for v in &volumes {
            yaml.push_str(&format!("      - \"{}\"\n", escape_yaml_value(v)));
        }
    }

    if !wrote_any {
        return Ok(PathBuf::new());
    }

    let basename = workspace_folder
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace");
    let override_path = std::env::temp_dir().join(format!("bondar-{basename}-override.yml"));
    std::fs::write(&override_path, yaml).map_err(BondarError::Io)?;
    Ok(override_path)
}

fn compose_base_command(
    config: &DevContainerConfig,
    config_path: &Path,
    workspace_folder: &Path,
) -> Result<Command> {
    let mut cmd = Command::new("docker");
    cmd.arg("compose");
    for arg in compose_files_args(config, config_path, workspace_folder)? {
        cmd.arg(arg);
    }
    let override_path = write_compose_override(config, workspace_folder)?;
    if !override_path.as_os_str().is_empty() {
        cmd.arg("-f").arg(override_path);
    }
    Ok(cmd)
}

fn port_value_to_string(p: &crate::config::PortValue) -> String {
    match p {
        crate::config::PortValue::Number(n) => n.to_string(),
        crate::config::PortValue::Text(s) => s.clone(),
    }
}

fn escape_yaml_value(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        // `$$` renders a literal `$` in compose, preventing `${VAR}` expansion
        .replace('$', "$$")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn escape_yaml_key(input: &str) -> String {
    if !input.is_empty()
        && input
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        input.to_string()
    } else {
        format!("\"{}\"", escape_yaml_value(input))
    }
}

pub fn compose_up(
    config: &DevContainerConfig,
    config_path: &Path,
    workspace_folder: &Path,
    remove_existing: bool,
    no_build: bool,
) -> Result<()> {
    println!("Starting Docker Compose services...");
    let mut cmd = compose_base_command(config, config_path, workspace_folder)?;
    cmd.arg("up");
    cmd.arg("-d");
    if remove_existing {
        cmd.arg("--force-recreate");
    }
    if no_build {
        cmd.arg("--no-build");
    }
    let mut seen_services = std::collections::HashSet::new();
    for s in &config.run_services {
        if seen_services.insert(s.clone()) {
            cmd.arg(s);
        } else {
            eprintln!("Warning: duplicate runServices entry '{s}', skipping");
        }
    }
    cmd.current_dir(workspace_folder);
    cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    let status = cmd
        .status()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker compose up: {e}")))?;
    if !status.success() {
        return Err(BondarError::Docker("docker compose up failed".to_string()));
    }
    Ok(())
}

pub fn compose_down(
    config: &DevContainerConfig,
    config_path: &Path,
    workspace_folder: &Path,
) -> Result<()> {
    // shutdownAction semantics for compose:
    // - unset (default): tear down services (remove)
    // - "none": do nothing
    // - "stopCompose": stop services but keep them
    let shutdown = config.shutdown_action.as_deref().unwrap_or("remove");
    if shutdown == "none" {
        println!("shutdownAction is 'none', skipping compose down");
        return Ok(());
    }

    let action = if shutdown == "stopCompose" {
        "stop"
    } else {
        "down"
    };

    // "stop" fails when no container exists; skip it instead
    if action == "stop" {
        let (exists, _) = service_container_state(config, config_path, workspace_folder)?;
        if !exists {
            println!("Service container does not exist, skipping 'docker compose stop'");
            return Ok(());
        }
    }

    println!("Running 'docker compose {action}'...");
    let mut cmd = compose_base_command(config, config_path, workspace_folder)?;
    cmd.arg(action);
    cmd.current_dir(workspace_folder);
    cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    let status = cmd
        .status()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker compose {action}: {e}")))?;
    if !status.success() {
        return Err(BondarError::Docker(format!(
            "docker compose {action} failed"
        )));
    }
    Ok(())
}

pub fn get_service_container_id(
    config: &DevContainerConfig,
    config_path: &Path,
    workspace_folder: &Path,
) -> Result<String> {
    let service = config
        .service
        .as_deref()
        .ok_or_else(|| BondarError::Config("No service specified".to_string()))?;
    let mut cmd = compose_base_command(config, config_path, workspace_folder)?;
    cmd.arg("ps");
    cmd.arg("-q");
    cmd.arg(service);
    cmd.current_dir(workspace_folder);
    let output = cmd
        .output()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker compose ps: {e}")))?;
    if !output.status.success() {
        return Err(BondarError::Docker("docker compose ps failed".to_string()));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let id = stdout.lines().next().unwrap_or("").trim().to_string();
    if id.is_empty() {
        return Err(BondarError::Docker(format!(
            "Service {service} container not found"
        )));
    }
    Ok(id)
}

/// Whether the service container already exists, and if so, whether it is running.
pub fn service_container_state(
    config: &DevContainerConfig,
    config_path: &Path,
    workspace_folder: &Path,
) -> Result<(bool, bool)> {
    let id = match get_service_container_id(config, config_path, workspace_folder) {
        Ok(id) => id,
        Err(_) => return Ok((false, false)),
    };
    let output = Command::new("docker")
        .args(["inspect", "--format", "{{.State.Running}}", &id])
        .output()
        .map_err(|e| BondarError::Docker(format!("Failed to inspect container: {e}")))?;
    if !output.status.success() {
        return Ok((true, false));
    }
    let running = String::from_utf8_lossy(&output.stdout).trim() == "true";
    Ok((true, running))
}

pub fn get_service_container_name(
    config: &DevContainerConfig,
    config_path: &Path,
    workspace_folder: &Path,
) -> Result<String> {
    let id = get_service_container_id(config, config_path, workspace_folder)?;
    let output = Command::new("docker")
        .args(["inspect", "--format", "{{.Name}}", &id])
        .output()
        .map_err(|e| BondarError::Docker(format!("Failed to inspect container: {e}")))?;
    if !output.status.success() {
        return Ok(id);
    }
    let name = String::from_utf8_lossy(&output.stdout)
        .trim()
        .trim_start_matches('/')
        .to_string();
    if name.is_empty() { Ok(id) } else { Ok(name) }
}

pub fn compose_exec(
    config: &DevContainerConfig,
    config_path: &Path,
    workspace_folder: &Path,
    user: Option<&str>,
    workdir: Option<&str>,
    env: Option<&std::collections::HashMap<String, String>>,
    command: &[String],
) -> Result<()> {
    let service = config.service.as_deref().ok_or_else(|| {
        BondarError::Config(
            "'service' must be specified in devcontainer.json when using dockerComposeFile"
                .to_string(),
        )
    })?;
    let mut cmd = compose_base_command(config, config_path, workspace_folder)?;
    cmd.arg("exec");
    if let Some(u) = user {
        cmd.arg("--user").arg(u);
    }
    if let Some(w) = workdir {
        cmd.arg("-w").arg(w);
    }
    if let Some(env_map) = env {
        // Resolve ${containerEnv:KEY} references against the containerEnv map
        let container_env_map: std::collections::HashMap<String, String> = config
            .container_env
            .iter()
            .map(|(k, v)| {
                let target = workdir.unwrap_or("/");
                (
                    k.clone(),
                    crate::docker::expand_vars_for_host_with_target(v, workspace_folder, target),
                )
            })
            .collect();
        for (k, v) in env_map {
            let target = workdir.unwrap_or("/");
            let from_map =
                crate::docker::expand_container_env_from_map(v, &container_env_map, None);
            let expanded = crate::docker::expand_vars_for_host_with_target(
                &from_map,
                workspace_folder,
                target,
            );
            cmd.arg("-e").arg(format!("{k}={expanded}"));
        }
    }
    // Handle TTY
    let is_tty = std::io::IsTerminal::is_terminal(&std::io::stdin())
        && std::io::IsTerminal::is_terminal(&std::io::stdout());
    if !is_tty {
        cmd.arg("-T");
    }
    cmd.arg(service);
    cmd.args(command);
    cmd.current_dir(workspace_folder);
    let status = cmd
        .status()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker compose exec: {e}")))?;
    if !status.success() {
        let code = status.code().unwrap_or(1);
        crate::lifecycle::reap_children();
        std::process::exit(code);
    }
    Ok(())
}

pub fn check_compose_available() -> Result<()> {
    let output = Command::new("docker")
        .args(["compose", "version"])
        .output()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker compose: {e}")))?;
    if !output.status.success() {
        return Err(BondarError::Docker(
            "docker compose not available".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MountObject;
    use std::collections::HashMap;

    #[test]
    fn test_mount_string_to_compose_volume_basic() {
        assert_eq!(
            mount_string_to_compose_volume("type=bind,source=/host,target=/container,readonly"),
            Some("/host:/container:ro".to_string())
        );
    }

    #[test]
    fn test_mount_string_to_compose_volume_volume() {
        assert_eq!(
            mount_string_to_compose_volume("type=volume,source=myvol,target=/data"),
            Some("myvol:/data".to_string())
        );
    }

    #[test]
    fn test_mount_string_to_compose_volume_ro_flag() {
        assert_eq!(
            mount_string_to_compose_volume("type=bind,source=/a,target=/b,ro"),
            Some("/a:/b:ro".to_string())
        );
        assert_eq!(
            mount_string_to_compose_volume("type=bind,source=/a,target=/b"),
            Some("/a:/b".to_string())
        );
    }

    #[test]
    fn test_mount_string_missing_target() {
        assert_eq!(mount_string_to_compose_volume("type=bind,source=/a"), None);
    }

    #[test]
    fn test_mount_string_anonymous_volume() {
        // source-less volume mount -> target only (anonymous volume)
        assert_eq!(
            mount_string_to_compose_volume("type=volume,target=/data"),
            Some("/data".to_string())
        );
        assert_eq!(
            mount_string_to_compose_volume("type=volume,target=/data,ro"),
            Some("/data:ro".to_string())
        );
    }

    #[test]
    fn test_mount_string_tmpfs_not_supported() {
        assert_eq!(
            mount_string_to_compose_volume("type=tmpfs,target=/data"),
            None
        );
    }

    #[test]
    fn test_mount_string_relative_bind_source() {
        // Conversion still succeeds for relative sources (a warning is
        // emitted about the named-volume interpretation)
        assert_eq!(
            mount_string_to_compose_volume("type=bind,source=data,target=/data"),
            Some("data:/data".to_string())
        );
        assert_eq!(
            mount_string_to_compose_volume("type=bind,source=./data,target=/data"),
            Some("./data:/data".to_string())
        );
    }

    #[test]
    fn test_escape_yaml_value() {
        assert_eq!(escape_yaml_value("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
        assert_eq!(escape_yaml_value("plain"), "plain");
        assert_eq!(escape_yaml_value("a\tb\rc"), "a\\tb\\rc");
        assert_eq!(escape_yaml_value(""), "");
        assert_eq!(escape_yaml_value("a$b"), "a$$b");
    }

    #[test]
    fn test_escape_yaml_key() {
        assert_eq!(escape_yaml_key("MY_VAR-1"), "MY_VAR-1");
        assert_eq!(escape_yaml_key("weird key"), "\"weird key\"");
        assert_eq!(escape_yaml_key(""), "\"\"");
        assert_eq!(escape_yaml_key("123"), "123");
        assert_eq!(escape_yaml_key("key:with:colon"), "\"key:with:colon\"");
    }

    #[test]
    fn test_escape_yaml_value_dollar() {
        assert_eq!(escape_yaml_value("a${VAR}b"), "a$${VAR}b");
        assert_eq!(escape_yaml_value("plain"), "plain");
        assert_eq!(escape_yaml_value("a\"b$"), "a\\\"b$$");
    }

    #[test]
    fn test_tmpfs_object_mount_skipped_in_override() {
        let dir = std::env::temp_dir().join("bondar-ovr-test4");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = DevContainerConfig {
            service: Some("app".to_string()),
            mounts: vec![MountValue::Object(MountObject {
                source: None,
                target: Some("/tmp".to_string()),
                mount_type: Some("tmpfs".to_string()),
                readonly: None,
            })],
            ..Default::default()
        };
        let path = write_compose_override(&cfg, &dir).unwrap();
        assert!(path.as_os_str().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_port_value_to_string() {
        assert_eq!(
            port_value_to_string(&crate::config::PortValue::Number(8080)),
            "8080"
        );
        assert_eq!(
            port_value_to_string(&crate::config::PortValue::Text("3000:3001".to_string())),
            "3000:3001"
        );
    }

    #[test]
    fn test_write_compose_override_ports_and_volumes() {
        let dir = std::env::temp_dir().join("bondar-ovr-test3");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = DevContainerConfig {
            service: Some("app".to_string()),
            forward_ports: vec![crate::config::ForwardPort::Number(8080)],
            mounts: vec![MountValue::Object(MountObject {
                source: Some("/host".to_string()),
                target: Some("/data".to_string()),
                mount_type: Some("bind".to_string()),
                readonly: Some(true),
            })],
            ..Default::default()
        };
        let path = write_compose_override(&cfg, &dir).unwrap();
        assert!(!path.as_os_str().is_empty());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("- \"0.0.0.0:8080:8080\""));
        assert!(content.contains("- \"/host:/data:ro\""));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_compose_override_secrets() {
        let dir = std::env::temp_dir().join("bondar-ovr-test4");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe {
            std::env::set_var("BONDAR_TEST_OVR_SECRET", "sv");
        }
        let cfg = DevContainerConfig {
            service: Some("app".to_string()),
            secrets: Some(HashMap::from([(
                "SEC".to_string(),
                serde_json::json!({"localEnv": "BONDAR_TEST_OVR_SECRET"}),
            )])),
            ..Default::default()
        };
        let path = write_compose_override(&cfg, &dir).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("SEC: \"sv\""));
        unsafe {
            std::env::remove_var("BONDAR_TEST_OVR_SECRET");
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_compose_override_env() {
        let dir = std::env::temp_dir().join("bondar-ovr-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let cfg = DevContainerConfig {
            service: Some("app".to_string()),
            container_env: env,
            forward_ports: vec![crate::config::ForwardPort::Number(8080)],
            ..Default::default()
        };
        let path = write_compose_override(&cfg, &dir).unwrap();
        assert!(!path.as_os_str().is_empty());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("  app:"));
        assert!(content.contains("FOO: \"bar\""));
        assert!(content.contains("- \"0.0.0.0:8080:8080\""));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_compose_override_empty() {
        let dir = std::env::temp_dir().join("bondar-ovr-test2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = DevContainerConfig {
            service: Some("app".to_string()),
            ..Default::default()
        };
        let path = write_compose_override(&cfg, &dir).unwrap();
        assert!(path.as_os_str().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_compose_files_args_expansion() {
        let dir = std::env::temp_dir().join("bondar-compose-files");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".devcontainer")).unwrap();
        std::fs::write(dir.join("docker-compose.yml"), "services: {}").unwrap();
        let cfg = DevContainerConfig {
            docker_compose_file: Some(ComposeFileValue::Single(
                "${localWorkspaceFolder}/docker-compose.yml".to_string(),
            )),
            ..Default::default()
        };
        let cfg_path = dir.join(".devcontainer/devcontainer.json");
        let args = compose_files_args(&cfg, &cfg_path, &dir).unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-f");
        // ${localWorkspaceFolder} expands to the workspace root
        assert!(args[1].ends_with("docker-compose.yml"));
        assert!(args[1].starts_with(&dir.to_string_lossy().to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_compose_files_args_multiple() {
        let dir = std::env::temp_dir().join("bondar-compose-multi");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".devcontainer")).unwrap();
        std::fs::write(dir.join("a.yml"), "services: {}").unwrap();
        std::fs::write(dir.join("b.yml"), "services: {}").unwrap();
        let cfg = DevContainerConfig {
            docker_compose_file: Some(ComposeFileValue::Multiple(vec![
                "a.yml".to_string(),
                "b.yml".to_string(),
            ])),
            ..Default::default()
        };
        let cfg_path = dir.join(".devcontainer/devcontainer.json");
        let args = compose_files_args(&cfg, &cfg_path, &dir).unwrap();
        assert_eq!(args.len(), 4);
        assert_eq!(args[0], "-f");
        assert!(args[1].ends_with("a.yml"));
        assert_eq!(args[2], "-f");
        assert!(args[3].ends_with("b.yml"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_compose_files_args_for_build() {
        let dir = std::env::temp_dir().join("bondar-compose-build");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".devcontainer")).unwrap();
        std::fs::write(dir.join("docker-compose.yml"), "services: {}").unwrap();
        let cfg = DevContainerConfig {
            docker_compose_file: Some(ComposeFileValue::Single("docker-compose.yml".to_string())),
            ..Default::default()
        };
        let cfg_path = dir.join(".devcontainer/devcontainer.json");
        let args = compose_files_args_for_build(&cfg, &cfg_path, &dir).unwrap();
        assert_eq!(args.len(), 2);
        assert!(args[1].ends_with("docker-compose.yml"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
