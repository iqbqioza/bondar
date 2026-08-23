use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::{BondarError, Result};

/// Parameters for executing a command inside the dev container.
#[derive(Clone, Copy)]
pub struct ContainerExec<'a> {
    pub container_name: &'a str,
    pub user: Option<&'a str>,
    pub workdir: &'a str,
    pub workspace_folder: &'a Path,
    pub env: Option<&'a HashMap<String, String>>,
}

pub fn execute_host_lifecycle(value: &serde_json::Value, workspace_folder: &Path) -> Result<()> {
    execute_value_with_env(value, workspace_folder, None, None)
}

pub fn execute_container_lifecycle_with_env(
    value: &serde_json::Value,
    container_name: &str,
    user: Option<&str>,
    workdir: &str,
    workspace_folder: &Path,
    env: Option<&HashMap<String, String>>,
) -> Result<()> {
    let exec = ContainerExec {
        container_name,
        user,
        workdir,
        workspace_folder,
        env,
    };
    execute_value_with_env(value, workspace_folder, Some(&exec), None)
}

fn execute_value_with_env(
    value: &serde_json::Value,
    workspace_folder: &Path,
    container: Option<&ContainerExec<'_>>,
    _label: Option<&str>,
) -> Result<()> {
    match value {
        serde_json::Value::String(s) => {
            execute_single_command_with_env(s, &[], true, workspace_folder, container)?;
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                return Ok(());
            }
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if parts.is_empty() {
                return Err(BondarError::Config(
                    "Invalid lifecycle array command".to_string(),
                ));
            }
            let cmd = &parts[0];
            let args: Vec<&str> = parts[1..].iter().map(String::as_str).collect();
            execute_single_command_with_env(cmd, &args, false, workspace_folder, container)?;
        }
        serde_json::Value::Object(map) => {
            for (key, cmd_val) in map {
                println!("Running lifecycle '{key}'...");
                execute_value_with_env(cmd_val, workspace_folder, container, Some(key))?;
            }
        }
        serde_json::Value::Null => {}
        _ => {
            return Err(BondarError::Config(format!(
                "Invalid lifecycle command type: {value}"
            )));
        }
    }
    Ok(())
}

fn execute_single_command_with_env(
    cmd: &str,
    args: &[&str],
    use_shell: bool,
    workspace_folder: &Path,
    container: Option<&ContainerExec<'_>>,
) -> Result<()> {
    if let Some(exec) = container {
        execute_in_container_with_env(cmd, args, use_shell, exec)
    } else {
        execute_on_host(cmd, args, use_shell, workspace_folder)
    }
}

fn execute_on_host(
    cmd: &str,
    args: &[&str],
    use_shell: bool,
    workspace_folder: &Path,
) -> Result<()> {
    let expanded_cmd = crate::docker::expand_vars_for_host(cmd, workspace_folder);
    let expanded_args: Vec<String> = args
        .iter()
        .map(|a| crate::docker::expand_vars_for_host(a, workspace_folder))
        .collect();

    println!(
        "Executing on host: {expanded_cmd} {}",
        expanded_args.join(" ")
    );

    let mut command = if use_shell {
        let full = if expanded_args.is_empty() {
            expanded_cmd.clone()
        } else {
            format!("{expanded_cmd} {}", expanded_args.join(" "))
        };
        if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(&full);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg(&full);
            c
        }
    } else {
        let mut c = Command::new(&expanded_cmd);
        c.args(&expanded_args);
        c
    };

    command.current_dir(workspace_folder);
    command.stdout(Stdio::inherit()).stderr(Stdio::inherit());

    let status = command
        .status()
        .map_err(|e| BondarError::Config(format!("Failed to execute host command: {e}")))?;

    if !status.success() {
        return Err(BondarError::Config(format!(
            "Host command failed: {expanded_cmd} (exit {})",
            status.code().unwrap_or(-1)
        )));
    }

    Ok(())
}

fn execute_in_container_with_env(
    cmd: &str,
    args: &[&str],
    use_shell: bool,
    exec: &ContainerExec<'_>,
) -> Result<()> {
    let mut docker_cmd = build_container_command(cmd, args, use_shell, exec)?;
    docker_cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());

    let status = docker_cmd
        .status()
        .map_err(|e| BondarError::Docker(format!("Failed to exec in container: {e}")))?;

    if !status.success() {
        return Err(BondarError::Docker(format!(
            "Container command failed: {cmd} (exit {})",
            status.code().unwrap_or(-1)
        )));
    }

    Ok(())
}

pub fn spawn_container_lifecycle(
    value: &serde_json::Value,
    container_name: &str,
    user: Option<&str>,
    workdir: &str,
    workspace_folder: &Path,
    env: Option<&HashMap<String, String>>,
) -> Result<()> {
    let exec = ContainerExec {
        container_name,
        user,
        workdir,
        workspace_folder,
        env,
    };
    spawn_container_value(value, &exec)
}

fn spawn_container_value(value: &serde_json::Value, exec: &ContainerExec<'_>) -> Result<()> {
    match value {
        serde_json::Value::String(s) => {
            spawn_container_command(s, &[], true, exec)?;
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                return Ok(());
            }
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if parts.is_empty() {
                return Err(BondarError::Config(
                    "Invalid lifecycle array command".to_string(),
                ));
            }
            let cmd = &parts[0];
            let args: Vec<&str> = parts[1..].iter().map(String::as_str).collect();
            spawn_container_command(cmd, &args, false, exec)?;
        }
        serde_json::Value::Object(map) => {
            for (key, cmd_val) in map {
                println!("Spawning lifecycle '{key}' in background...");
                spawn_container_value(cmd_val, exec)?;
            }
        }
        serde_json::Value::Null => {}
        _ => {
            return Err(BondarError::Config(format!(
                "Invalid lifecycle command type: {value}"
            )));
        }
    }
    Ok(())
}

fn spawn_container_command(
    cmd: &str,
    args: &[&str],
    use_shell: bool,
    exec: &ContainerExec<'_>,
) -> Result<()> {
    println!(
        "Spawning in container {} (background): {cmd} {}",
        exec.container_name,
        args.join(" ")
    );
    let mut docker_cmd = build_container_command(cmd, args, use_shell, exec)?;
    docker_cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    docker_cmd
        .spawn()
        .map_err(|e| BondarError::Docker(format!("Failed to spawn docker exec: {e}")))?;
    Ok(())
}

fn build_container_command(
    cmd: &str,
    args: &[&str],
    use_shell: bool,
    exec: &ContainerExec<'_>,
) -> Result<Command> {
    let expanded_cmd =
        crate::docker::expand_vars_for_container(cmd, exec.workspace_folder, exec.workdir);
    let expanded_args: Vec<String> = args
        .iter()
        .map(|a| crate::docker::expand_vars_for_container(a, exec.workspace_folder, exec.workdir))
        .collect();

    let mut docker_cmd = Command::new("docker");
    docker_cmd.arg("exec");

    if let Some(u) = exec.user {
        docker_cmd.arg("--user").arg(u);
    }

    if !exec.workdir.is_empty() {
        docker_cmd.arg("-w").arg(exec.workdir);
    }

    if let Some(env_map) = exec.env {
        for (k, v) in env_map {
            docker_cmd.arg("-e").arg(format!("{k}={v}"));
        }
    }

    docker_cmd.arg(exec.container_name);

    if use_shell {
        let full = if expanded_args.is_empty() {
            expanded_cmd.clone()
        } else {
            format!("{expanded_cmd} {}", expanded_args.join(" "))
        };
        docker_cmd.arg("sh").arg("-c").arg(&full);
    } else {
        docker_cmd.arg(&expanded_cmd);
        docker_cmd.args(&expanded_args);
    }

    Ok(docker_cmd)
}

pub fn lifecycle_summary(config: &crate::config::DevContainerConfig) -> Vec<String> {
    let mut cmds = Vec::new();
    if config.initialize_command.is_some() {
        cmds.push("initializeCommand".to_string());
    }
    if config.on_create_command.is_some() {
        cmds.push("onCreateCommand".to_string());
    }
    if config.update_content_command.is_some() {
        cmds.push("updateContentCommand".to_string());
    }
    if config.post_create_command.is_some() {
        cmds.push("postCreateCommand".to_string());
    }
    if config.post_start_command.is_some() {
        cmds.push("postStartCommand".to_string());
    }
    if config.post_attach_command.is_some() {
        cmds.push("postAttachCommand".to_string());
    }
    cmds
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_lifecycle_summary() {
        let cfg = crate::config::DevContainerConfig {
            initialize_command: Some(json!("echo hi")),
            post_create_command: Some(json!(["echo", "hi"])),
            ..Default::default()
        };
        let summary = lifecycle_summary(&cfg);
        assert_eq!(summary.len(), 2);
        assert!(summary.contains(&"initializeCommand".to_string()));
    }

    #[test]
    fn test_lifecycle_summary_all() {
        let cfg = crate::config::DevContainerConfig {
            initialize_command: Some(json!("a")),
            on_create_command: Some(json!("b")),
            update_content_command: Some(json!("c")),
            post_create_command: Some(json!("d")),
            post_start_command: Some(json!("e")),
            post_attach_command: Some(json!("f")),
            ..Default::default()
        };
        let summary = lifecycle_summary(&cfg);
        assert_eq!(summary.len(), 6);
    }

    #[test]
    fn test_lifecycle_summary_empty() {
        let cfg = crate::config::DevContainerConfig::default();
        assert!(lifecycle_summary(&cfg).is_empty());
    }
}
