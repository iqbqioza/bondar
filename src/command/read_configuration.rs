use std::path::PathBuf;

use crate::config;
use crate::docker;
use crate::error::Result;

pub fn run(workspace_folder: Option<PathBuf>, config_path: Option<PathBuf>) -> Result<()> {
    let ws = docker::get_workspace_folder(workspace_folder)?;
    let (cfg, cfg_path) = config::load_config(&ws, config_path.as_deref())?;

    println!("Config file: {}", cfg_path.display());
    println!("Workspace: {}", ws.display());
    println!();

    let json = serde_json::to_string_pretty(&cfg).unwrap_or_else(|_| "{}".to_string());
    println!("{json}");
    println!();

    let summary = crate::lifecycle::lifecycle_summary(&cfg);
    if !summary.is_empty() {
        println!("Lifecycle: {}", summary.join(", "));
    }
    if let Some(req) = &cfg.host_requirements {
        println!("Host requirements: {req}");
    }
    if let Some(ports) = &cfg.ports_attributes {
        println!("Ports attributes: {ports}");
    }
    if cfg.docker_compose_file.is_some() {
        println!(
            "Mode: Docker Compose (service: {})",
            cfg.service.as_deref().unwrap_or("unknown")
        );
    } else if cfg.build.is_some() {
        println!("Mode: Build");
    } else {
        println!(
            "Mode: Image ({})",
            cfg.image.as_deref().unwrap_or("unknown")
        );
    }

    Ok(())
}
