use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::config;
use crate::docker;
use crate::error::{BondarError, Result};

pub fn run(
    workspace_folder: Option<PathBuf>,
    config_path: Option<PathBuf>,
    follow: bool,
    tail: Option<String>,
) -> Result<()> {
    docker::check_docker_available()?;

    let ws = docker::get_workspace_folder(workspace_folder)?;
    let (cfg, cfg_path) = config::load_config(&ws, config_path.as_deref())?;

    if cfg.docker_compose_file.is_some() {
        let mut cmd = Command::new("docker");
        cmd.arg("compose");
        for arg in crate::compose::compose_files_args_for_build(&cfg, &cfg_path, &ws)? {
            cmd.arg(arg);
        }
        cmd.arg("logs");
        if follow {
            cmd.arg("-f");
        }
        if let Some(t) = tail {
            cmd.arg("--tail").arg(t);
        }
        if let Some(service) = &cfg.service {
            cmd.arg(service);
        }
        cmd.current_dir(&ws);
        cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        let status = cmd
            .status()
            .map_err(|e| BondarError::Docker(format!("Failed to run compose logs: {e}")))?;
        if !status.success() {
            return Err(BondarError::Docker(
                "docker compose logs failed".to_string(),
            ));
        }
        return Ok(());
    }

    let container_name = cfg.container_name(&ws);
    if !docker::container_exists(&container_name)? {
        return Err(BondarError::NotFound(format!(
            "Container {container_name} does not exist"
        )));
    }

    let mut cmd = Command::new("docker");
    cmd.arg("logs");
    if follow {
        cmd.arg("-f");
    }
    if let Some(t) = tail {
        cmd.arg("--tail").arg(t);
    }
    cmd.arg(&container_name);
    cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    let status = cmd
        .status()
        .map_err(|e| BondarError::Docker(format!("Failed to run docker logs: {e}")))?;
    if !status.success() {
        return Err(BondarError::Docker("docker logs failed".to_string()));
    }
    Ok(())
}
