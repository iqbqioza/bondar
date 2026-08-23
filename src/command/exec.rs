use std::path::PathBuf;

use crate::config;
use crate::docker;
use crate::error::Result;

pub fn run(
    workspace_folder: Option<PathBuf>,
    config_path: Option<PathBuf>,
    user: Option<String>,
    workdir: Option<String>,
    command: Vec<String>,
) -> Result<()> {
    docker::check_docker_available()?;

    let ws = docker::get_workspace_folder(workspace_folder)?;
    let (cfg, _) = config::load_config(&ws, config_path.as_deref())?;

    let container_name = cfg.container_name(&ws);
    let exec_user = user
        .or_else(|| cfg.remote_user.clone())
        .or_else(|| cfg.container_user.clone());
    let exec_workdir = workdir.or(cfg.workspace_folder.clone());

    let remote_env = if cfg.remote_env.is_empty() {
        None
    } else {
        Some(&cfg.remote_env)
    };
    docker::exec_in_container(
        &container_name,
        exec_user.as_deref(),
        exec_workdir.as_deref(),
        &command,
        remote_env,
        Some(&ws),
    )?;

    Ok(())
}
