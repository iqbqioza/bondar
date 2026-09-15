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
    let container_name = cfg.container_name(&ws);

    // Never stop/remove a container that belongs to a different workspace
    // (same-basename name collision). Reuse a single existence check.
    let exists = docker::container_exists(&container_name)?;

    // The image metadata may declare shutdownAction when the config does not
    let shutdown = match cfg.shutdown_action.clone() {
        Some(action) => action,
        None => {
            let mut from_metadata = None;
            if exists && let Some(raw) = docker::container_metadata_label(&container_name) {
                from_metadata = crate::features::image_metadata_shutdown_action(&raw);
            }
            from_metadata.unwrap_or_else(|| "remove".to_string())
        }
    };
    let shutdown = shutdown.as_str();
    if shutdown != "none" && exists {
        docker::ensure_container_matches_workspace(&container_name, &ws)?;
    }

    match shutdown {
        "none" => {
            println!("shutdownAction is 'none', skipping down (container kept)");
        }
        "stopContainer" => {
            if !exists {
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
            if !exists {
                println!("Container {container_name} does not exist");
                return Ok(());
            }
            println!("Removing container {container_name}...");
            docker::remove_container(&container_name)?;
            println!("Container {container_name} removed");
        }
        _ => {
            // "remove" is the default; anything else is already rejected by
            // config::validate but handled defensively here
            if shutdown != "remove" {
                eprintln!("Warning: unknown shutdownAction '{shutdown}'; treating as remove");
            }
            if !exists {
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
