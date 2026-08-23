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

    // shutdownAction semantics:
    // - unset (default): remove the container (bondar down = teardown)
    // - "none": do nothing (keep container running)
    // - "stopContainer": stop the container but keep it
    let shutdown = cfg.shutdown_action.as_deref().unwrap_or("remove");
    let container_name = cfg.container_name(&ws);

    match shutdown {
        "none" => {
            println!("shutdownAction is 'none', skipping down (container kept)");
        }
        "stopContainer" => {
            if !docker::container_exists(&container_name)? {
                println!("Container {container_name} does not exist");
                return Ok(());
            }
            println!("Stopping container {container_name} (shutdownAction: stopContainer)...");
            docker::stop_container(&container_name)?;
            println!("Container {container_name} stopped (kept for reuse)");
        }
        "stopCompose" => {
            eprintln!(
                "Warning: shutdownAction 'stopCompose' requires dockerComposeFile; treating as remove"
            );
            if !docker::container_exists(&container_name)? {
                println!("Container {container_name} does not exist");
                return Ok(());
            }
            println!("Removing container {container_name}...");
            docker::remove_container(&container_name)?;
            println!("Container {container_name} removed");
        }
        _ => {
            if !docker::container_exists(&container_name)? {
                println!("Container {container_name} does not exist");
                return Ok(());
            }
            println!("Removing container {container_name}...");
            docker::remove_container(&container_name)?;
            println!("Container {container_name} removed");
        }
    }

    Ok(())
}
