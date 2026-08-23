use std::path::Path;
use std::process::{Command, Stdio};

use crate::config::{ComposeFileValue, DevContainerConfig};
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

fn compose_base_command(config: &DevContainerConfig, config_path: &Path) -> Result<Command> {
    let mut cmd = Command::new("docker");
    cmd.arg("compose");
    for arg in compose_files_args(config, config_path)? {
        cmd.arg(arg);
    }
    Ok(cmd)
}

pub fn compose_up(
    config: &DevContainerConfig,
    config_path: &Path,
    workspace_folder: &Path,
) -> Result<()> {
    println!("Starting Docker Compose services...");
    let mut cmd = compose_base_command(config, config_path)?;
    cmd.arg("up");
    cmd.arg("-d");
    if let Some(services) = config.extra.get("runServices").and_then(|v| v.as_array()) {
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
    let shutdown = config.shutdown_action.as_deref().unwrap_or("stopCompose");
    if shutdown == "none" {
        println!("shutdownAction is 'none', skipping compose down");
        return Ok(());
    }

    println!("Stopping Docker Compose services...");
    let mut cmd = compose_base_command(config, config_path)?;
    cmd.arg("down");
    cmd.current_dir(workspace_folder);
    cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    let status = cmd
        .status()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker compose down: {e}")))?;
    if !status.success() {
        return Err(BondarError::Docker(
            "docker compose down failed".to_string(),
        ));
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
    let mut cmd = compose_base_command(config, config_path)?;
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
    let mut cmd = compose_base_command(config, config_path)?;
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
