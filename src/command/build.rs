use std::path::PathBuf;

use crate::config;
use crate::docker;
use crate::error::Result;

pub fn run(
    workspace_folder: Option<PathBuf>,
    config_path: Option<PathBuf>,
    no_cache: bool,
) -> Result<()> {
    docker::check_docker_available()?;

    let ws = docker::get_workspace_folder(workspace_folder)?;
    let (cfg, cfg_path) = config::load_config(&ws, config_path.as_deref())?;

    if cfg.docker_compose_file.is_some() {
        println!("Building Docker Compose services...");
        crate::compose::check_compose_available()?;

        let mut build_cmd = std::process::Command::new("docker");
        build_cmd.arg("compose");
        for arg in crate::compose::compose_files_args_for_build(&cfg, &cfg_path)? {
            build_cmd.arg(arg);
        }
        build_cmd.arg("build");
        if no_cache {
            build_cmd.arg("--no-cache");
        }
        build_cmd.current_dir(&ws);
        build_cmd.stdout(std::process::Stdio::inherit());
        build_cmd.stderr(std::process::Stdio::inherit());
        let status = build_cmd.status().map_err(|e| {
            crate::error::BondarError::Docker(format!("Failed to run compose build: {e}"))
        })?;
        if !status.success() {
            return Err(crate::error::BondarError::Docker(
                "docker compose build failed".to_string(),
            ));
        }
        println!("Compose build completed");
        return Ok(());
    }

    if cfg.build.is_none() {
        println!("No build configured, image: {:?}", cfg.image);
        return Ok(());
    }

    let image_name = docker::resolve_image_name(&cfg, &cfg_path, &ws)?;
    docker::build_image(&cfg, &cfg_path, &ws, &image_name, no_cache)?;

    println!("Build completed: {image_name}");
    Ok(())
}
