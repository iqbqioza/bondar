use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::{DevContainerConfig, MountValue};
use crate::error::{BondarError, Result};

pub fn check_docker_available() -> Result<()> {
    // Distinguish a missing CLI from an unreachable daemon
    let cli_ok = Command::new("docker")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !cli_ok {
        return Err(BondarError::Docker(
            "Docker CLI not found in PATH; install Docker to use bondar".to_string(),
        ));
    }

    let output = Command::new("docker")
        .arg("version")
        .arg("--format")
        .arg("{{.Server.Version}}")
        .output()
        .map_err(|e| BondarError::Docker(format!("Failed to execute docker: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BondarError::Docker(format!(
            "Docker daemon not reachable: {stderr}"
        )));
    }
    Ok(())
}

pub fn build_image(
    config: &DevContainerConfig,
    config_path: &Path,
    workspace_folder: &Path,
    image_name: &str,
    no_cache: bool,
) -> Result<()> {
    let build = config
        .build
        .as_ref()
        .ok_or_else(|| BondarError::Config("No 'build' section found, cannot build".to_string()))?;

    let config_dir = config_path.parent().unwrap_or(workspace_folder);
    let dockerfile = build
        .dockerfile
        .clone()
        .unwrap_or_else(|| "Dockerfile".to_string());
    let dockerfile = expand_vars_for_host(&dockerfile, workspace_folder);
    let dockerfile_path = config_dir.join(&dockerfile);

    if !dockerfile_path.exists() {
        return Err(BondarError::NotFound(format!(
            "Dockerfile not found: {}",
            dockerfile_path.display()
        )));
    }

    let context = build
        .context
        .as_ref()
        .map(|c| {
            let expanded = expand_vars_for_host(c, workspace_folder);
            config_dir.join(&expanded)
        })
        .unwrap_or_else(|| config_dir.to_path_buf());

    let context_str = context.to_string_lossy().to_string();
    let dockerfile_str = dockerfile_path.to_string_lossy().to_string();

    let mut cmd = Command::new("docker");
    cmd.arg("build");
    cmd.arg("-f").arg(&dockerfile_str);
    cmd.arg("-t").arg(image_name);

    for (k, v) in &build.args {
        let expanded = expand_vars_for_host(v, workspace_folder);
        if expanded.is_empty() {
            eprintln!(
                "Warning: build arg '{k}' resolved to an empty value (unset variable?); skipping"
            );
            continue;
        }
        cmd.arg("--build-arg").arg(format!("{k}={expanded}"));
    }

    if let Some(target) = &build.target {
        cmd.arg("--target").arg(target);
    }

    for opt in &build.options {
        let expanded = expand_vars_for_host(opt, workspace_folder);
        cmd.arg(&expanded);
    }

    if let Some(cache_from) = &build.cache_from {
        match cache_from {
            crate::config::CacheFromValue::Single(s) => {
                let expanded = expand_vars_for_host(s, workspace_folder);
                cmd.arg("--cache-from").arg(expanded);
            }
            crate::config::CacheFromValue::Multiple(vec) => {
                for s in vec {
                    let expanded = expand_vars_for_host(s, workspace_folder);
                    cmd.arg("--cache-from").arg(expanded);
                }
            }
        }
    }

    if no_cache {
        cmd.arg("--no-cache");
    }

    cmd.arg(&context_str);
    cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());

    println!("Building image {image_name}...");
    println!("  Dockerfile: {}", dockerfile_path.display());
    println!("  Context: {}", context.display());

    let status = cmd
        .status()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker build: {e}")))?;

    if !status.success() {
        return Err(BondarError::Docker("docker build failed".to_string()));
    }

    Ok(())
}

pub fn resolve_image_name(config: &DevContainerConfig, workspace_folder: &Path) -> Result<String> {
    if config.build.is_some() {
        let base = if let Some(name) = &config.name {
            let sanitized: String = name
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .collect();
            format!("bondar-{sanitized}")
        } else {
            let basename = workspace_folder
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("workspace");
            let sanitized: String = basename
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .collect();
            format!("bondar-{sanitized}")
        };
        Ok(base)
    } else if let Some(image) = &config.image {
        Ok(image.clone())
    } else {
        Err(BondarError::Config(
            "No image or build specified".to_string(),
        ))
    }
}

pub fn container_exists(name: &str) -> Result<bool> {
    let output = Command::new("docker")
        .args(["ps", "-a", "--format", "{{.Names}}"])
        .output()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker ps: {e}")))?;

    if !output.status.success() {
        return Err(BondarError::Docker("docker ps failed".to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().any(|line| line.trim() == name))
}

pub fn container_running(name: &str) -> Result<bool> {
    let output = Command::new("docker")
        .args(["ps", "--format", "{{.Names}}"])
        .output()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker ps: {e}")))?;

    if !output.status.success() {
        return Err(BondarError::Docker("docker ps failed".to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().any(|line| line.trim() == name))
}

/// Single `docker ps -a` call to learn both existence and running state.
pub fn container_exists_and_running(name: &str) -> Result<(bool, bool)> {
    let output = Command::new("docker")
        .args(["ps", "-a", "--format", "{{.Names}}\t{{.Status}}"])
        .output()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker ps: {e}")))?;

    if !output.status.success() {
        return Err(BondarError::Docker("docker ps failed".to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let mut parts = line.split('\t');
        let Some(n) = parts.next() else {
            continue;
        };
        if n.trim() == name {
            let status = parts.next().unwrap_or("");
            return Ok((true, status.starts_with("Up")));
        }
    }
    Ok((false, false))
}

pub fn find_containers_for_workspace(workspace_folder: &Path) -> Result<Vec<String>> {
    let label_value = workspace_folder.display().to_string();
    // Docker filters split on ',' and '='; bail out silently for exotic paths
    if label_value.contains(',') || label_value.contains('=') {
        eprintln!(
            "Warning: workspace path contains ',' or '='; cannot search for existing containers by label"
        );
        return Ok(Vec::new());
    }
    let output = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("label=devcontainer.local_folder={label_value}"),
            "--format",
            "{{.Names}}",
        ])
        .output()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker ps: {e}")))?;

    if !output.status.success() {
        return Err(BondarError::Docker("docker ps failed".to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

pub fn remove_container(name: &str) -> Result<()> {
    let status = Command::new("docker")
        .args(["rm", "-f", name])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker rm: {e}")))?;

    if !status.success() {
        return Err(BondarError::Docker(format!(
            "Failed to remove container {name}"
        )));
    }
    Ok(())
}

pub fn stop_container(name: &str) -> Result<()> {
    let status = Command::new("docker")
        .args(["stop", name])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker stop: {e}")))?;

    if !status.success() {
        return Err(BondarError::Docker(format!(
            "Failed to stop container {name}"
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn create_and_start_container(
    config: &DevContainerConfig,
    workspace_folder: &Path,
    config_path: &Path,
    container_name: &str,
    image_name: &str,
    remove_existing: bool,
) -> Result<()> {
    let (exists, running) = container_exists_and_running(container_name)?;
    if exists {
        if remove_existing {
            println!("Removing existing container {container_name}...");
            remove_container(container_name)?;
        } else if running {
            println!("Container {container_name} is already running");
            return Ok(());
        } else {
            println!("Starting existing container {container_name}...");
            let status = Command::new("docker")
                .args(["start", container_name])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .map_err(|e| BondarError::Docker(format!("Failed to run docker start: {e}")))?;
            if !status.success() {
                return Err(BondarError::Docker(format!(
                    "Failed to start container {container_name}"
                )));
            }
            return Ok(());
        }
    }

    let workspace_folder_str = workspace_folder.to_string_lossy().to_string();
    let workspace_target = config.workspace_folder_or_default();

    let mut cmd = Command::new("docker");
    cmd.arg("run");
    cmd.arg("-d");
    cmd.arg("--name").arg(container_name);

    let use_init = config.init.unwrap_or(false) || config.override_command.unwrap_or(true);
    if use_init {
        cmd.arg("--init");
    }

    if config.privileged.unwrap_or(false) {
        cmd.arg("--privileged");
    }

    for cap in &config.cap_add {
        cmd.arg("--cap-add").arg(cap);
    }

    for opt in &config.security_opt {
        cmd.arg("--security-opt").arg(opt);
    }

    for run_arg in &config.run_args {
        let expanded =
            expand_vars_for_host_with_target(run_arg, workspace_folder, &workspace_target);
        cmd.arg(&expanded);
    }

    // Labels for tracking
    cmd.arg("--label")
        .arg(format!("devcontainer.local_folder={workspace_folder_str}"));
    cmd.arg("--label").arg(format!(
        "devcontainer.config_file={}",
        config_path.display()
    ));
    let devcontainer_id = devcontainer_id_for(workspace_folder);
    cmd.arg("--label")
        .arg(format!("devcontainer.id={devcontainer_id}"));

    if let Some(attrs) = &config.ports_attributes {
        let json_str = serde_json::to_string(attrs).unwrap_or_default();
        cmd.arg("--label")
            .arg(format!("devcontainer.ports_attributes={json_str}"));
    }
    if let Some(other) = &config.other_ports_attributes {
        let json_str = serde_json::to_string(other).unwrap_or_default();
        cmd.arg("--label")
            .arg(format!("devcontainer.other_ports_attributes={json_str}"));
    }

    // Workspace mount
    if let Some(mount) = &config.workspace_mount {
        let expanded = expand_vars_for_host_with_target(mount, workspace_folder, &workspace_target);
        cmd.arg("--mount").arg(&expanded);
    } else {
        let mount = format!(
            "type=bind,source={workspace_folder_str},target={workspace_target},consistency=cached"
        );
        cmd.arg("--mount").arg(&mount);
    }
    // Always set working dir to the workspace folder
    cmd.arg("-w").arg(&workspace_target);

    // Additional mounts
    for m in &config.mounts {
        match m {
            MountValue::String(s) => {
                let expanded =
                    expand_vars_for_host_with_target(s, workspace_folder, &workspace_target);
                cmd.arg("--mount").arg(expanded);
            }
            MountValue::Object(obj) => {
                let mut parts = Vec::new();
                if let Some(t) = &obj.mount_type {
                    parts.push(format!("type={t}"));
                }
                if let Some(s) = &obj.source {
                    parts.push(format!(
                        "source={}",
                        expand_vars_for_host_with_target(s, workspace_folder, &workspace_target)
                    ));
                }
                if let Some(t) = &obj.target {
                    parts.push(format!(
                        "target={}",
                        expand_vars_for_host_with_target(t, workspace_folder, &workspace_target)
                    ));
                }
                if obj.readonly.unwrap_or(false) {
                    parts.push("readonly".to_string());
                }
                if !parts.is_empty() {
                    cmd.arg("--mount").arg(parts.join(","));
                }
            }
        }
    }

    // Env - containerEnv is set on the container; remoteEnv applies only at exec time
    for (k, v) in &config.container_env {
        let expanded_v = expand_vars_for_host_with_target(v, workspace_folder, &workspace_target);
        cmd.arg("-e").arg(format!("{k}={expanded_v}"));
    }

    // Secrets resolved from local env (devcontainer spec: { "MY_SECRET": { "localEnv": "VAR" } })
    for (k, v) in resolve_secrets(config) {
        cmd.arg("-e").arg(format!("{k}={v}"));
    }

    // Publish / forward ports
    for port in &config.forward_ports {
        let port_str = match port {
            crate::config::ForwardPort::Number(n) => n.to_string(),
            crate::config::ForwardPort::Text(s) => s.clone(),
        };
        if is_port_ignored(config, &port_str) {
            println!("Skipping forwardPorts '{port_str}' (onAutoForward: ignore)");
            continue;
        }
        if let Some(mut publish) = publish_port_arg(&port_str) {
            if is_udp_port(config, &port_str) {
                publish.push_str("/udp");
            }
            cmd.arg("-p").arg(publish);
        } else {
            eprintln!(
                "Warning: forwardPorts '{port_str}' references a service host, cannot publish with docker run"
            );
        }
    }

    if let Some(app_port) = &config.app_port {
        let ports: Vec<String> = match app_port {
            crate::config::AppPortValue::Single(p) => vec![port_value_to_string(p)],
            crate::config::AppPortValue::Multiple(v) => {
                v.iter().map(port_value_to_string).collect()
            }
        };
        for p in ports {
            if is_port_ignored(config, &p) {
                println!("Skipping appPort '{p}' (onAutoForward: ignore)");
                continue;
            }
            if let Some(mut publish) = publish_port_arg(&p) {
                if is_udp_port(config, &p) {
                    publish.push_str("/udp");
                }
                cmd.arg("-p").arg(publish);
            } else {
                eprintln!(
                    "Warning: appPort '{p}' references a service host, cannot publish with docker run"
                );
            }
        }
    }

    // User
    if let Some(user) = &config.container_user {
        cmd.arg("--user").arg(user);
    }

    // Image
    cmd.arg(image_name);

    // Override command
    if config.override_command.unwrap_or(true) {
        cmd.arg("sh");
        cmd.arg("-c");
        cmd.arg("while sleep 1000; do :; done");
    }

    cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());

    println!("Creating container {container_name} from {image_name}...");
    let status = cmd
        .status()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker run: {e}")))?;

    if !status.success() {
        return Err(BondarError::Docker("docker run failed".to_string()));
    }

    Ok(())
}

fn port_value_to_string(p: &crate::config::PortValue) -> String {
    match p {
        crate::config::PortValue::Number(n) => n.to_string(),
        crate::config::PortValue::Text(s) => s.clone(),
    }
}

pub fn is_udp_port(config: &DevContainerConfig, port_spec: &str) -> bool {
    // Determine the container port portion of the spec
    let container_port = port_spec.rsplit(':').next().unwrap_or(port_spec);
    // Explicit per-port attributes take precedence
    if let Some(attrs) = &config.ports_attributes
        && let Some(entry) = attrs.get(container_port)
        && let Some(obj) = entry.as_object()
        && obj.get("protocol").and_then(|v| v.as_str()) == Some("udp")
    {
        return true;
    }
    // Fall back to the default attributes
    if let Some(other) = &config.other_ports_attributes
        && let Some(obj) = other.as_object()
        && obj.get("protocol").and_then(|v| v.as_str()) == Some("udp")
    {
        return true;
    }
    false
}

/// Whether `onAutoForward: "ignore"` disables publishing for the port.
pub fn is_port_ignored(config: &DevContainerConfig, port_spec: &str) -> bool {
    let container_port = port_spec.rsplit(':').next().unwrap_or(port_spec);
    if let Some(attrs) = &config.ports_attributes
        && let Some(entry) = attrs.get(container_port)
        && let Some(obj) = entry.as_object()
        && obj.get("onAutoForward").and_then(|v| v.as_str()) == Some("ignore")
    {
        return true;
    }
    if let Some(other) = &config.other_ports_attributes
        && let Some(obj) = other.as_object()
        && obj.get("onAutoForward").and_then(|v| v.as_str()) == Some("ignore")
    {
        return true;
    }
    false
}

pub fn publish_port_arg(spec: &str) -> Option<String> {
    // Preserve an explicit /udp protocol suffix if present
    let (base, protocol) = match spec.strip_suffix("/udp") {
        Some(b) => (b, "/udp"),
        None => (spec, ""),
    };
    if base.starts_with('[') {
        return publish_ipv6_arg(base).map(|p| format!("{p}{protocol}"));
    }
    publish_port_arg_inner(base).map(|p| format!("{p}{protocol}"))
}

/// Publish an IPv6 bind address in bracket form: `[::1]:8080` or `[::1]:8080:8080`.
fn publish_ipv6_arg(spec: &str) -> Option<String> {
    let end = spec.find(']')?;
    let addr = &spec[1..end];
    if addr.is_empty() || addr.contains('[') {
        return None;
    }
    let rest = &spec[end + 1..];
    let ports: Vec<&str> = rest.split(':').filter(|s| !s.is_empty()).collect();
    match ports.len() {
        1 => {
            if is_port_or_range(ports[0]) {
                Some(format!("[{addr}]:{}:{}", ports[0], ports[0]))
            } else {
                None
            }
        }
        2 => {
            if is_port_or_range(ports[0]) && is_port_or_range(ports[1]) {
                Some(format!("[{addr}]:{}:{}", ports[0], ports[1]))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn publish_port_arg_inner(spec: &str) -> Option<String> {
    if spec.is_empty() {
        return None;
    }
    // No colon: single port or range -> host:container with the same spec
    if !spec.contains(':') {
        if is_port_or_range(spec) {
            return Some(format!("{spec}:{spec}"));
        }
        return None;
    }
    let mut parts = spec.split(':');
    let first = parts.next().unwrap_or("");
    let rest = parts.collect::<Vec<_>>();
    // "host:container"
    if rest.len() == 1 {
        let host_is_ip = is_ipv4(first);
        let host_is_number = is_port_or_range(first);
        let container_ok = is_port_or_range(rest[0]);
        if !container_ok {
            // empty or non-numeric container port ("8080:" / "8080:abc")
            return None;
        }
        if host_is_ip {
            // "127.0.0.1:9090" -> "127.0.0.1:9090:9090"
            return Some(format!("{spec}:{}", rest[0]));
        }
        if host_is_number {
            // "8080:80" -> "8080:80" (also ranges "8080-8085:8080-8085")
            return Some(spec.to_string());
        }
        // "db:5432" -> service host, cannot publish
        return None;
    }
    // "ip:host:container"
    if rest.len() == 2 {
        let ip_ok = is_ipv4(first);
        if ip_ok && is_port_or_range(rest[0]) && is_port_or_range(rest[1]) {
            return Some(spec.to_string());
        }
    }
    None
}

fn is_port_or_range(s: &str) -> bool {
    if s.parse::<u16>().is_ok() {
        return true;
    }
    if let Some((a, b)) = s.split_once('-') {
        return a.parse::<u16>().is_ok() && b.parse::<u16>().is_ok();
    }
    false
}

fn is_ipv4(s: &str) -> bool {
    let octets: Vec<&str> = s.split('.').collect();
    octets.len() == 4 && octets.iter().all(|x| x.parse::<u8>().is_ok())
}

pub fn expand_vars_for_host(input: &str, workspace_folder: &Path) -> String {
    expand_vars_for_host_with_target(input, workspace_folder, "/workspace")
}

pub fn expand_vars_for_host_with_target(
    input: &str,
    workspace_folder: &Path,
    container_workspace: &str,
) -> String {
    let mut result = input.to_string();
    let ws = workspace_folder.to_string_lossy().to_string();
    let basename = workspace_folder
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    result = result.replace("${localWorkspaceFolder}", &ws);
    result = result.replace("${localWorkspaceFolderBasename}", &basename);
    result = result.replace("${containerWorkspaceFolder}", container_workspace);
    let container_basename = container_workspace
        .rsplit('/')
        .next()
        .unwrap_or("workspace")
        .to_string();
    result = result.replace("${containerWorkspaceFolderBasename}", &container_basename);

    result = expand_local_env_vars(&result);
    result = expand_container_env_vars(&result);
    result = expand_devcontainer_id(&result, workspace_folder);
    result
}

pub fn expand_vars_for_container(
    input: &str,
    workspace_folder: &Path,
    container_workspace: &str,
) -> String {
    let mut result = input.to_string();
    let ws = workspace_folder.to_string_lossy().to_string();
    let basename = workspace_folder
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let container_basename = container_workspace
        .rsplit('/')
        .next()
        .unwrap_or("workspace")
        .to_string();

    result = result.replace("${localWorkspaceFolder}", &ws);
    result = result.replace("${localWorkspaceFolderBasename}", &basename);
    result = result.replace("${containerWorkspaceFolder}", container_workspace);
    result = result.replace("${containerWorkspaceFolderBasename}", &container_basename);

    result = expand_local_env_vars(&result);
    result = expand_container_env_vars(&result);
    result = expand_devcontainer_id(&result, workspace_folder);
    result
}

fn devcontainer_id_for(workspace_folder: &Path) -> String {
    let ws_str = workspace_folder.to_string_lossy().to_string();
    let mut hash: u64 = 14695981039346656037;
    for b in ws_str.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("{hash:016x}")
}

fn expand_devcontainer_id(input: &str, workspace_folder: &Path) -> String {
    if !input.contains("${devcontainerId}") {
        return input.to_string();
    }
    let id = devcontainer_id_for(workspace_folder);
    input.replace("${devcontainerId}", &id)
}

fn expand_container_env_vars(input: &str) -> String {
    let mut result = String::new();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next();
            let mut var_content = String::new();
            let mut found_end = false;
            for nc in chars.by_ref() {
                if nc == '}' {
                    found_end = true;
                    break;
                }
                var_content.push(nc);
            }
            if found_end && var_content.starts_with("containerEnv:") {
                let rest = &var_content["containerEnv:".len()..];
                let (var_name, default_val) = if let Some(colon_pos) = rest.find(':') {
                    (&rest[..colon_pos], Some(&rest[colon_pos + 1..]))
                } else {
                    (rest, None)
                };
                if var_name.is_empty() {
                    eprintln!(
                        "Warning: '${{containerEnv:}}' has an empty variable name, resolved to empty"
                    );
                }
                let env_val = std::env::var(var_name)
                    .unwrap_or_else(|_| default_val.unwrap_or("").to_string());
                result.push_str(&env_val);
            } else if found_end {
                result.push_str("${");
                result.push_str(&var_content);
                result.push('}');
            } else {
                result.push_str("${");
                result.push_str(&var_content);
            }
        } else {
            result.push(c);
        }
    }

    result
}

fn expand_local_env_vars(input: &str) -> String {
    let mut result = String::new();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut var_content = String::new();
            let mut found_end = false;
            for nc in chars.by_ref() {
                if nc == '}' {
                    found_end = true;
                    break;
                }
                var_content.push(nc);
            }
            if found_end && var_content.starts_with("localEnv:") {
                let rest = &var_content["localEnv:".len()..];
                let (var_name, default_val) = if let Some(colon_pos) = rest.find(':') {
                    (&rest[..colon_pos], Some(&rest[colon_pos + 1..]))
                } else {
                    (rest, None)
                };
                if var_name.is_empty() {
                    eprintln!(
                        "Warning: '${{localEnv:}}' has an empty variable name, resolved to empty"
                    );
                }
                let env_val = std::env::var(var_name)
                    .unwrap_or_else(|_| default_val.unwrap_or("").to_string());
                result.push_str(&env_val);
            } else if found_end {
                result.push_str("${");
                result.push_str(&var_content);
                result.push('}');
            } else {
                result.push_str("${");
                result.push_str(&var_content);
            }
        } else {
            result.push(c);
        }
    }

    result
}

pub fn resolve_secrets(config: &DevContainerConfig) -> Vec<(String, String)> {
    let Some(secrets) = &config.secrets else {
        return Vec::new();
    };
    let mut resolved = Vec::new();
    for (key, spec) in secrets {
        match spec {
            serde_json::Value::Object(map) => {
                let var_name = match map.get("localEnv") {
                    Some(v) => match v.as_str() {
                        Some(s) => s.to_string(),
                        None => {
                            eprintln!(
                                "Warning: secret '{key}' localEnv must be a string, got {v}; using the secret name as the variable name"
                            );
                            key.clone()
                        }
                    },
                    None => key.clone(),
                };
                if let Ok(value) = std::env::var(&var_name) {
                    resolved.push((key.clone(), value));
                } else {
                    eprintln!(
                        "Warning: secret '{key}' references unset localEnv variable '{var_name}'"
                    );
                }
            }
            serde_json::Value::String(path) => {
                eprintln!(
                    "Warning: secret '{key}' uses the file path form '{path}' which is not supported; use {{\"localEnv\": \"VAR\"}} instead"
                );
            }
            other => {
                eprintln!("Warning: secret '{key}' has unsupported format: {other}");
            }
        }
    }
    resolved
}

pub fn exec_in_container(
    container_name: &str,
    user: Option<&str>,
    workdir: Option<&str>,
    command: &[String],
    env: Option<&HashMap<String, String>>,
    workspace_folder: Option<&Path>,
) -> Result<()> {
    if !container_running(container_name)? {
        return Err(BondarError::Docker(format!(
            "Container {container_name} is not running; run 'bondar up' first"
        )));
    }

    let mut cmd = Command::new("docker");
    let is_tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if is_tty {
        cmd.arg("exec").arg("-it");
    } else {
        cmd.arg("exec").arg("-i");
    }

    if let Some(u) = user {
        cmd.arg("--user").arg(u);
    }

    if let Some(w) = workdir {
        cmd.arg("-w").arg(w);
    }

    if let Some(env_map) = env {
        for (k, v) in env_map {
            let expanded_v = if let Some(ws) = workspace_folder {
                let target = workdir.unwrap_or("/workspace");
                expand_vars_for_host_with_target(v, ws, target)
            } else {
                v.clone()
            };
            cmd.arg("-e").arg(format!("{k}={expanded_v}"));
        }
    }

    cmd.arg(container_name);
    cmd.args(command);

    let status = cmd
        .status()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker exec: {e}")))?;

    if !status.success() {
        let code = status.code().unwrap_or(1);
        std::process::exit(code);
    }

    Ok(())
}

pub fn get_workspace_folder(provided: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = provided {
        if p.is_dir() {
            return Ok(p.canonicalize()?);
        }
        return Err(BondarError::NotFound(format!(
            "Workspace folder not found or not a directory: {}",
            p.display()
        )));
    }

    let cwd = std::env::current_dir()?;
    Ok(cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_publish_port_arg_number() {
        assert_eq!(publish_port_arg("8080"), Some("8080:8080".to_string()));
    }

    #[test]
    fn test_publish_port_arg_host_container() {
        assert_eq!(publish_port_arg("8080:80"), Some("8080:80".to_string()));
    }

    #[test]
    fn test_publish_port_arg_ip() {
        assert_eq!(
            publish_port_arg("127.0.0.1:9090"),
            Some("127.0.0.1:9090:9090".to_string())
        );
    }

    #[test]
    fn test_publish_port_arg_ip_host_container() {
        assert_eq!(
            publish_port_arg("127.0.0.1:8080:80"),
            Some("127.0.0.1:8080:80".to_string())
        );
    }

    #[test]
    fn test_publish_port_arg_service_name() {
        assert_eq!(publish_port_arg("db:5432"), None);
        assert_eq!(publish_port_arg("not-a-port"), None);
    }

    #[test]
    fn test_publish_port_arg_empty_or_invalid_container_port() {
        assert_eq!(publish_port_arg("8080:"), None);
        assert_eq!(publish_port_arg("8080:abc"), None);
        assert_eq!(publish_port_arg(":8080"), None);
    }

    #[test]
    fn test_publish_port_arg_partial_ip_rejected() {
        // 3-octet "IP" is invalid; should not be treated as an IP
        assert_eq!(publish_port_arg("1.2.3:80"), None);
    }

    #[test]
    fn test_is_ipv4() {
        assert!(is_ipv4("127.0.0.1"));
        assert!(is_ipv4("0.0.0.0"));
        assert!(!is_ipv4("1.2.3"));
        assert!(!is_ipv4("8080"));
        assert!(!is_ipv4("a.b.c.d"));
    }

    #[test]
    fn test_is_ipv4_octet_range() {
        // Octets must be 0-255
        assert!(!is_ipv4("999.999.999.999"));
        assert!(!is_ipv4("256.0.0.1"));
        assert!(is_ipv4("255.255.255.255"));
    }

    #[test]
    fn test_publish_port_arg_range() {
        assert_eq!(
            publish_port_arg("8080-8085"),
            Some("8080-8085:8080-8085".to_string())
        );
        assert_eq!(
            publish_port_arg("8080-8085:8080-8085"),
            Some("8080-8085:8080-8085".to_string())
        );
        assert_eq!(
            publish_port_arg("127.0.0.1:8080-8085"),
            Some("127.0.0.1:8080-8085:8080-8085".to_string())
        );
        // Host range with single container port
        assert_eq!(
            publish_port_arg("8080-8085:8080"),
            Some("8080-8085:8080".to_string())
        );
    }

    #[test]
    fn test_is_port_or_range() {
        assert!(is_port_or_range("8080"));
        assert!(is_port_or_range("8080-8085"));
        assert!(!is_port_or_range("8080-"));
        assert!(!is_port_or_range("-8080"));
        assert!(!is_port_or_range("abc"));
        assert!(!is_port_or_range("8080-8085-8090"));
    }

    #[test]
    fn test_publish_ipv6_arg() {
        assert_eq!(
            publish_port_arg("[::1]:8080"),
            Some("[::1]:8080:8080".to_string())
        );
        assert_eq!(
            publish_port_arg("[::1]:8080:8080"),
            Some("[::1]:8080:8080".to_string())
        );
        assert_eq!(
            publish_port_arg("[2001:db8::1]:8080:80"),
            Some("[2001:db8::1]:8080:80".to_string())
        );
        assert_eq!(
            publish_port_arg("[::]:8080"),
            Some("[::]:8080:8080".to_string())
        );
        // Invalid forms
        assert_eq!(publish_port_arg("[::1]"), None);
        assert_eq!(publish_port_arg("[]:8080"), None);
        assert_eq!(publish_port_arg("[::1]:abc"), None);
        assert_eq!(publish_port_arg("[::1]:8080:abc"), None);
    }

    #[test]
    fn test_expand_vars_for_host_with_target() {
        let ws = std::path::Path::new("/home/user/proj");
        let expanded = expand_vars_for_host_with_target(
            "${localWorkspaceFolder}|${localWorkspaceFolderBasename}|${containerWorkspaceFolder}|${containerWorkspaceFolderBasename}",
            ws,
            "/myws",
        );
        assert_eq!(expanded, "/home/user/proj|proj|/myws|myws");
    }

    #[test]
    fn test_devcontainer_id_for_stable() {
        let ws = std::path::Path::new("/home/user/proj");
        let a = devcontainer_id_for(ws);
        let b = devcontainer_id_for(ws);
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
        let other = devcontainer_id_for(std::path::Path::new("/home/user/other"));
        assert_ne!(a, other);
    }

    #[test]
    fn test_expand_vars_devcontainer_id() {
        let ws = std::path::Path::new("/home/user/proj");
        let id = devcontainer_id_for(ws);
        let expanded = expand_vars_for_host_with_target("id=${devcontainerId}", ws, "/workspace");
        assert_eq!(expanded, format!("id={id}"));
    }

    #[test]
    fn test_expand_vars_for_host_default_target() {
        let ws = std::path::Path::new("/home/user/proj");
        let expanded =
            expand_vars_for_host("${localWorkspaceFolder}|${containerWorkspaceFolder}", ws);
        assert_eq!(expanded, "/home/user/proj|/workspace");
    }

    #[test]
    fn test_expand_devcontainer_id_pass_through() {
        let ws = std::path::Path::new("/home/user/proj");
        assert_eq!(expand_devcontainer_id("no id here", ws), "no id here");
    }

    #[test]
    fn test_expand_local_env_vars_default() {
        assert_eq!(
            expand_local_env_vars("${localEnv:UNSET_VAR_XYZ_123:fallback}"),
            "fallback"
        );
        assert_eq!(expand_local_env_vars("${localEnv:UNSET_VAR_XYZ_123}"), "");
        assert_eq!(
            expand_local_env_vars("pre${localEnv:UNSET_VAR_XYZ_123:def}post"),
            "predefpost"
        );
    }

    #[test]
    fn test_expand_container_env_vars_default() {
        assert_eq!(
            expand_container_env_vars("${containerEnv:UNSET_VAR_XYZ_123:fallback}"),
            "fallback"
        );
        assert_eq!(
            expand_container_env_vars("${containerEnv:UNSET_VAR_XYZ_123}"),
            ""
        );
        assert_eq!(
            expand_container_env_vars("${containerEnv:UNSET_VAR_XYZ_123:}"),
            ""
        );
    }

    #[test]
    fn test_expand_env_vars_empty_default() {
        assert_eq!(expand_local_env_vars("${localEnv:UNSET_VAR_XYZ_123:}"), "");
        assert_eq!(
            expand_container_env_vars("${containerEnv:UNSET_VAR_XYZ_123:}"),
            ""
        );
    }

    #[test]
    fn test_is_udp_port() {
        let cfg = DevContainerConfig {
            ports_attributes: Some(serde_json::json!({
                "3000": {"protocol": "udp"},
                "8080": {"protocol": "tcp"}
            })),
            ..Default::default()
        };
        assert!(is_udp_port(&cfg, "3000"));
        assert!(is_udp_port(&cfg, "127.0.0.1:3000"));
        assert!(!is_udp_port(&cfg, "8080"));
        assert!(!is_udp_port(&cfg, "9090"));
        let plain = DevContainerConfig::default();
        assert!(!is_udp_port(&plain, "3000"));
    }

    #[test]
    fn test_is_port_ignored() {
        let cfg = DevContainerConfig {
            ports_attributes: Some(serde_json::json!({
                "3000": {"onAutoForward": "ignore"},
                "8080": {"onAutoForward": "notify"}
            })),
            ..Default::default()
        };
        assert!(is_port_ignored(&cfg, "3000"));
        assert!(is_port_ignored(&cfg, "127.0.0.1:3000"));
        assert!(!is_port_ignored(&cfg, "8080"));
        assert!(!is_port_ignored(&cfg, "9090"));
        assert!(!is_port_ignored(&DevContainerConfig::default(), "3000"));
    }

    #[test]
    fn test_is_udp_port_other_attributes_fallback() {
        let cfg = DevContainerConfig {
            other_ports_attributes: Some(serde_json::json!({
                "protocol": "udp"
            })),
            ..Default::default()
        };
        assert!(is_udp_port(&cfg, "9090"));
        assert!(is_udp_port(&cfg, "127.0.0.1:9090"));
    }

    #[test]
    fn test_is_port_ignored_other_attributes_fallback() {
        let cfg = DevContainerConfig {
            other_ports_attributes: Some(serde_json::json!({
                "onAutoForward": "ignore"
            })),
            ..Default::default()
        };
        assert!(is_port_ignored(&cfg, "9090"));
        assert!(!is_port_ignored(&DevContainerConfig::default(), "9090"));
    }

    #[test]
    fn test_publish_port_arg_zero() {
        assert_eq!(publish_port_arg("0"), Some("0:0".to_string()));
        assert_eq!(publish_port_arg("0:0"), Some("0:0".to_string()));
    }

    #[test]
    fn test_expand_vars_complex() {
        let ws = std::path::Path::new("/home/user/proj");
        let expanded = expand_vars_for_host_with_target(
            "ws=${localWorkspaceFolder};id=${devcontainerId}",
            ws,
            "/myws",
        );
        let id = devcontainer_id_for(ws);
        assert_eq!(expanded, format!("ws=/home/user/proj;id={id}"));
    }

    #[test]
    fn test_resolve_secrets() {
        // Use a unique env name to avoid interference from parallel tests
        unsafe {
            std::env::set_var("BONDAR_TEST_SECRET_VAR", "secret-value");
        }
        let cfg = DevContainerConfig {
            secrets: Some(HashMap::from([
                (
                    "MY_SECRET".to_string(),
                    serde_json::json!({"localEnv": "BONDAR_TEST_SECRET_VAR"}),
                ),
                (
                    "FILE_SECRET".to_string(),
                    serde_json::json!("/run/secrets/x"),
                ),
            ])),
            ..Default::default()
        };
        let resolved = resolve_secrets(&cfg);
        // FILE_SECRET (file path form) is skipped with a warning
        assert_eq!(
            resolved,
            vec![("MY_SECRET".to_string(), "secret-value".to_string())]
        );
        unsafe {
            std::env::remove_var("BONDAR_TEST_SECRET_VAR");
        }
    }

    #[test]
    fn test_resolve_secrets_unset() {
        let cfg = DevContainerConfig {
            secrets: Some(HashMap::from([(
                "MISSING".to_string(),
                serde_json::json!({"localEnv": "BONDAR_TEST_UNSET_VAR"}),
            )])),
            ..Default::default()
        };
        let resolved = resolve_secrets(&cfg);
        assert!(resolved.is_empty());
    }

    #[test]
    fn test_expand_local_env_vars_real_env() {
        unsafe {
            std::env::set_var("BONDAR_TEST_ENV_VAR", "real-value");
        }
        let expanded = expand_local_env_vars("${localEnv:BONDAR_TEST_ENV_VAR}");
        assert_eq!(expanded, "real-value");
        unsafe {
            std::env::remove_var("BONDAR_TEST_ENV_VAR");
        }
    }

    #[test]
    fn test_expand_container_env_vars_real_env() {
        unsafe {
            std::env::set_var("BONDAR_TEST_CENV", "cv");
        }
        let expanded = expand_container_env_vars("${containerEnv:BONDAR_TEST_CENV}");
        assert_eq!(expanded, "cv");
        unsafe {
            std::env::remove_var("BONDAR_TEST_CENV");
        }
    }

    #[test]
    fn test_get_workspace_folder() {
        let dir = std::env::temp_dir().join("bondar-ws-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Existing directory -> canonicalized
        let got = get_workspace_folder(Some(dir.clone())).unwrap();
        assert_eq!(got, dir.canonicalize().unwrap());

        // Missing path -> error
        let missing = std::env::temp_dir().join("bondar-ws-missing");
        assert!(get_workspace_folder(Some(missing)).is_err());

        // File instead of directory -> error
        let file = std::env::temp_dir().join("bondar-ws-file.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(get_workspace_folder(Some(file.clone())).is_err());
        let _ = std::fs::remove_file(&file);

        // None -> current directory
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(get_workspace_folder(None).unwrap(), cwd);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_image_name() {
        // Build with name -> bondar-{sanitized-lowercase}
        let build_cfg = DevContainerConfig {
            name: Some("My Dev".to_string()),
            build: Some(crate::config::BuildConfig {
                dockerfile: Some("Dockerfile".to_string()),
                context: None,
                args: Default::default(),
                options: vec![],
                target: None,
                cache_from: None,
            }),
            ..Default::default()
        };
        assert_eq!(
            resolve_image_name(&build_cfg, std::path::Path::new("/tmp/x")).unwrap(),
            "bondar-my-dev"
        );

        // Build without name -> bondar-{basename}
        let build_unnamed = DevContainerConfig {
            build: Some(crate::config::BuildConfig {
                dockerfile: Some("Dockerfile".to_string()),
                context: None,
                args: Default::default(),
                options: vec![],
                target: None,
                cache_from: None,
            }),
            ..Default::default()
        };
        assert_eq!(
            resolve_image_name(&build_unnamed, std::path::Path::new("/tmp/my-proj")).unwrap(),
            "bondar-my-proj"
        );

        // Image -> as-is
        let image_cfg = DevContainerConfig {
            image: Some("ubuntu:22.04".to_string()),
            ..Default::default()
        };
        assert_eq!(
            resolve_image_name(&image_cfg, std::path::Path::new("/tmp/x")).unwrap(),
            "ubuntu:22.04"
        );

        // Neither -> error
        let empty = DevContainerConfig::default();
        assert!(resolve_image_name(&empty, std::path::Path::new("/tmp/x")).is_err());
    }

    #[test]
    fn test_publish_port_arg_udp_suffix() {
        assert_eq!(
            publish_port_arg("8080:8080/udp"),
            Some("8080:8080/udp".to_string())
        );
        assert_eq!(
            publish_port_arg("8080/udp"),
            Some("8080:8080/udp".to_string())
        );
    }

    #[test]
    fn test_port_value_to_string() {
        assert_eq!(
            port_value_to_string(&crate::config::PortValue::Number(8080)),
            "8080"
        );
        assert_eq!(
            port_value_to_string(&crate::config::PortValue::Text("db:5432".to_string())),
            "db:5432"
        );
    }

    #[test]
    fn test_expand_vars_for_container() {
        let ws = std::path::Path::new("/home/user/proj");
        let expanded = expand_vars_for_container(
            "${localWorkspaceFolder}|${localWorkspaceFolderBasename}|${containerWorkspaceFolder}|${containerWorkspaceFolderBasename}",
            ws,
            "/myws",
        );
        assert_eq!(expanded, "/home/user/proj|proj|/myws|myws");
    }
}
