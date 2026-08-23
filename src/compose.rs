use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::{ComposeFileValue, DevContainerConfig, MountValue};
use crate::error::{BondarError, Result};

pub fn compose_files_args_for_build(
    config: &DevContainerConfig,
    config_path: &Path,
) -> Result<Vec<String>> {
    compose_files_args(config, config_path)
}

fn compose_files_args(config: &DevContainerConfig, config_path: &Path) -> Result<Vec<String>> {
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
        let expanded = crate::docker::expand_vars_for_host(&f, config_dir);
        let path = config_dir.join(&expanded);
        let path_str = path.to_string_lossy().to_string();
        args.push("-f".to_string());
        args.push(path_str);
    }
    Ok(args)
}

fn mount_string_to_compose_volume(mount: &str) -> Option<String> {
    let mut mount_type = None;
    let mut source = None;
    let mut target = None;
    let mut readonly = false;
    for part in mount.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((key, value)) = part.split_once('=') {
            match key {
                "type" => mount_type = Some(value),
                "source" | "src" => source = Some(value),
                "target" | "dst" | "destination" => target = Some(value),
                "readonly" | "ro" => readonly = value == "true" || value == "1",
                _ => {}
            }
        } else {
            match part {
                "readonly" | "ro" => readonly = true,
                "bind" => mount_type = Some("bind"),
                _ => {}
            }
        }
    }
    let mount_type = mount_type.unwrap_or("volume");
    let source = source.unwrap_or_default();
    let target = target?;
    let mut vol = String::new();
    if !source.is_empty() {
        vol.push_str(source);
        vol.push(':');
    }
    vol.push_str(target);
    let _ = mount_type;
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
    yaml.push_str(&format!("  {service}:\n"));

    let has_env =
        !config.container_env.is_empty() || !config.secrets.as_ref().is_none_or(|s| s.is_empty());
    let mut ports: Vec<String> = Vec::new();
    for port in &config.forward_ports {
        let port_str = match port {
            crate::config::ForwardPort::Number(n) => n.to_string(),
            crate::config::ForwardPort::Text(s) => s.clone(),
        };
        if let Some(publish) = crate::docker::publish_port_arg(&port_str) {
            ports.push(format!("\"{publish}\""));
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
            if let Some(publish) = crate::docker::publish_port_arg(&p) {
                ports.push(format!("\"{publish}\""));
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
                    volumes.push(vol);
                }
            }
            MountValue::Object(obj) => {
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
                    volumes.push(vol);
                }
            }
        }
    }

    let mut wrote_any = false;
    if has_env {
        wrote_any = true;
        yaml.push_str("    environment:\n");
        for (k, v) in &config.container_env {
            let expanded = crate::docker::expand_vars_for_host_with_target(
                v,
                workspace_folder,
                &container_target,
            );
            yaml.push_str(&format!("      {k}: \"{expanded}\"\n"));
        }
        for (k, v) in crate::docker::resolve_secrets(config) {
            yaml.push_str(&format!("      {k}: \"{v}\"\n"));
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
            yaml.push_str(&format!("      - \"{v}\"\n"));
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
    for arg in compose_files_args(config, config_path)? {
        cmd.arg(arg);
    }
    if let Ok(override_path) = write_compose_override(config, workspace_folder)
        && !override_path.as_os_str().is_empty()
    {
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

pub fn compose_up(
    config: &DevContainerConfig,
    config_path: &Path,
    workspace_folder: &Path,
    remove_existing: bool,
) -> Result<()> {
    println!("Starting Docker Compose services...");
    let mut cmd = compose_base_command(config, config_path, workspace_folder)?;
    cmd.arg("up");
    cmd.arg("-d");
    if remove_existing {
        cmd.arg("--force-recreate");
    }
    for s in &config.run_services {
        cmd.arg(s);
    }
    if config.run_services.is_empty()
        && let Some(services) = config.extra.get("runServices").and_then(|v| v.as_array())
    {
        // Legacy fallback for configs parsed before run_services existed
        for s in services {
            if let Some(name) = s.as_str() {
                cmd.arg(name);
            }
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
    command: &[String],
) -> Result<()> {
    let service = config
        .service
        .as_deref()
        .ok_or_else(|| BondarError::Config("No service".to_string()))?;
    let mut cmd = compose_base_command(config, config_path, workspace_folder)?;
    cmd.arg("exec");
    if let Some(u) = user {
        cmd.arg("--user").arg(u);
    }
    if let Some(w) = workdir {
        cmd.arg("-w").arg(w);
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
}
