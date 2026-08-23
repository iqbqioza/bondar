use std::path::PathBuf;

use crate::config;
use crate::docker;
use crate::error::Result;

pub fn run(workspace_folder: Option<PathBuf>, config_path: Option<PathBuf>) -> Result<()> {
    docker::check_docker_available()?;

    let ws = docker::get_workspace_folder(workspace_folder)?;
    let (cfg, cfg_path) = config::load_config(&ws, config_path.as_deref())?;

    if cfg.docker_compose_file.is_some() {
        return crate::compose::compose_down(&cfg, &cfg_path, &ws);
    }

    let shutdown = cfg.shutdown_action.as_deref().unwrap_or("stopContainer");
    if shutdown == "none" {
        println!("shutdownAction is 'none', skipping down (container kept)");
        return Ok(());
    }

    let container_name = cfg.container_name(&ws);

    if !docker::container_exists(&container_name)? {
        println!("Container {container_name} does not exist");
        return Ok(());
    }

    println!("Removing container {container_name}...");
    docker::remove_container(&container_name)?;
    println!("Container {container_name} removed");

    Ok(())
}
