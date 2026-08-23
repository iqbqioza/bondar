use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::{BondarError, Result};

pub fn execute_host_lifecycle(value: &serde_json::Value, workspace_folder: &Path) -> Result<()> {
    execute_value(value, workspace_folder, None, None)
}

pub fn execute_container_lifecycle(
    value: &serde_json::Value,
    container_name: &str,
    user: Option<&str>,
    workdir: &str,
    workspace_folder: &Path,
) -> Result<()> {
    execute_container_lifecycle_with_env(
        value,
        container_name,
        user,
        workdir,
        workspace_folder,
        None,
    )
}

pub fn execute_container_lifecycle_with_env(
    value: &serde_json::Value,
    container_name: &str,
    user: Option<&str>,
    workdir: &str,
    workspace_folder: &Path,
    env: Option<&std::collections::HashMap<String, String>>,
) -> Result<()> {
    execute_value_with_env(
        value,
        workspace_folder,
        Some((container_name, user, workdir, env)),
        None,
    )
}

fn execute_value(
    value: &serde_json::Value,
    workspace_folder: &Path,
    container: Option<(&str, Option<&str>, &str)>,
    _label: Option<&str>,
) -> Result<()> {
    execute_value_with_env(
        value,
        workspace_folder,
        container.map(|(n, u, w)| (n, u, w, None)),
        _label,
    )
}

#[allow(clippy::type_complexity)]
fn execute_value_with_env(
    value: &serde_json::Value,
    workspace_folder: &Path,
    container: Option<(
        &str,
        Option<&str>,
        &str,
        Option<&std::collections::HashMap<String, String>>,
    )>,
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

#[allow(clippy::type_complexity)]
fn execute_single_command_with_env(
    cmd: &str,
    args: &[&str],
    use_shell: bool,
    workspace_folder: &Path,
    container: Option<(
        &str,
        Option<&str>,
        &str,
        Option<&std::collections::HashMap<String, String>>,
    )>,
) -> Result<()> {
    if let Some((container_name, user, workdir, env)) = container {
        execute_in_container_with_env(
            cmd,
            args,
            use_shell,
            container_name,
            user,
            workdir,
            workspace_folder,
            env,
        )
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
        let mut c = Command::new("sh");
        let full = if expanded_args.is_empty() {
            expanded_cmd.clone()
        } else {
            format!("{expanded_cmd} {}", expanded_args.join(" "))
        };
        c.arg("-c").arg(&full);
        c
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

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
fn execute_in_container_with_env(
    cmd: &str,
    args: &[&str],
    use_shell: bool,
    container_name: &str,
    user: Option<&str>,
    workdir: &str,
    workspace_folder: &Path,
    env: Option<&std::collections::HashMap<String, String>>,
) -> Result<()> {
    let mut docker_cmd = build_container_command(
        cmd,
        args,
        use_shell,
        container_name,
        user,
        workdir,
        workspace_folder,
        env,
    )?;
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
    env: Option<&std::collections::HashMap<String, String>>,
) -> Result<()> {
    spawn_container_value(value, container_name, user, workdir, workspace_folder, env)
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
fn spawn_container_value(
    value: &serde_json::Value,
    container_name: &str,
    user: Option<&str>,
    workdir: &str,
    workspace_folder: &Path,
    env: Option<&std::collections::HashMap<String, String>>,
) -> Result<()> {
    match value {
        serde_json::Value::String(s) => {
            spawn_container_command(
                s,
                &[],
                true,
                container_name,
                user,
                workdir,
                workspace_folder,
                env,
            )?;
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
            spawn_container_command(
                cmd,
                &args,
                false,
                container_name,
                user,
                workdir,
                workspace_folder,
                env,
            )?;
        }
        serde_json::Value::Object(map) => {
            for (key, cmd_val) in map {
                println!("Spawning lifecycle '{key}' in background...");
                spawn_container_value(
                    cmd_val,
                    container_name,
                    user,
                    workdir,
                    workspace_folder,
                    env,
                )?;
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

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
fn spawn_container_command(
    cmd: &str,
    args: &[&str],
    use_shell: bool,
    container_name: &str,
    user: Option<&str>,
    workdir: &str,
    workspace_folder: &Path,
    env: Option<&std::collections::HashMap<String, String>>,
) -> Result<()> {
    println!(
        "Spawning in container {container_name} (background): {cmd} {}",
        args.join(" ")
    );
    let mut docker_cmd = build_container_command(
        cmd,
        args,
        use_shell,
        container_name,
        user,
        workdir,
        workspace_folder,
        env,
    )?;
    docker_cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    docker_cmd
        .spawn()
        .map_err(|e| BondarError::Docker(format!("Failed to spawn docker exec: {e}")))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
fn build_container_command(
    cmd: &str,
    args: &[&str],
    use_shell: bool,
    container_name: &str,
    user: Option<&str>,
    workdir: &str,
    workspace_folder: &Path,
    env: Option<&std::collections::HashMap<String, String>>,
) -> Result<Command> {
    let expanded_cmd = crate::docker::expand_vars_for_container(cmd, workspace_folder, workdir);
    let expanded_args: Vec<String> = args
        .iter()
        .map(|a| crate::docker::expand_vars_for_container(a, workspace_folder, workdir))
        .collect();

    let mut docker_cmd = Command::new("docker");
    docker_cmd.arg("exec");

    if let Some(u) = user {
        docker_cmd.arg("--user").arg(u);
    }

    if !workdir.is_empty() {
        docker_cmd.arg("-w").arg(workdir);
    }

    if let Some(env_map) = env {
        for (k, v) in env_map {
            docker_cmd.arg("-e").arg(format!("{k}={v}"));
        }
    }

    docker_cmd.arg(container_name);

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
}
