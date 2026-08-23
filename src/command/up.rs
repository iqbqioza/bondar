use std::path::PathBuf;

use crate::config;
use crate::docker;
use crate::error::Result;

pub fn run(
    workspace_folder: Option<PathBuf>,
    config_path: Option<PathBuf>,
    remove_existing: bool,
    no_build: bool,
) -> Result<()> {
    docker::check_docker_available()?;

    let ws = docker::get_workspace_folder(workspace_folder)?;
    let (cfg, cfg_path) = config::load_config(&ws, config_path.as_deref())?;

    if cfg.docker_compose_file.is_some() {
        return Err(crate::error::BondarError::Config(
            "dockerComposeFile is not yet supported. Use image/build.".to_string(),
        ));
    }

    if cfg.extra.contains_key("features") {
        eprintln!("Warning: 'features' is not yet supported and will be ignored");
    }

    let image_name = docker::resolve_image_name(&cfg, &cfg_path, &ws)?;

    if cfg.build.is_some() && !no_build {
        // Only build if image doesn't exist or we always build
        // For simplicity, always build if build is configured
        docker::build_image(&cfg, &cfg_path, &ws, &image_name)?;
    }

    let container_name = cfg.container_name(&ws);

    docker::create_and_start_container(&cfg, &ws, &container_name, &image_name, remove_existing)?;

    println!("Container {container_name} is up");
    println!("  Image: {image_name}");
    println!("  Workspace: {}", ws.display());
    println!("  Use 'bondar exec -- <command>' to execute commands");
    println!("  Use 'bondar shell' for interactive shell");
    println!("  Use 'bondar down' to stop and remove");

    Ok(())
}
