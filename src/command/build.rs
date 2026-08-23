use std::path::PathBuf;

use crate::config;
use crate::docker;
use crate::error::Result;

pub fn run(
    workspace_folder: Option<PathBuf>,
    config_path: Option<PathBuf>,
    _no_cache: bool,
) -> Result<()> {
    docker::check_docker_available()?;

    let ws = docker::get_workspace_folder(workspace_folder)?;
    let (cfg, cfg_path) = config::load_config(&ws, config_path.as_deref())?;

    if cfg.build.is_none() {
        println!("No build configured, image: {:?}", cfg.image);
        return Ok(());
    }

    if cfg.extra.contains_key("features") {
        eprintln!("Warning: 'features' is not yet supported and will be ignored");
    }

    let image_name = docker::resolve_image_name(&cfg, &cfg_path, &ws)?;
    docker::build_image(&cfg, &cfg_path, &ws, &image_name)?;

    println!("Build completed: {image_name}");
    Ok(())
}
