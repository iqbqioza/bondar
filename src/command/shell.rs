use std::path::PathBuf;

use crate::config;
use crate::docker;
use crate::error::Result;

pub fn run(workspace_folder: Option<PathBuf>, config_path: Option<PathBuf>) -> Result<()> {
    docker::check_docker_available()?;

    let ws = docker::get_workspace_folder(workspace_folder)?;
    let (cfg, cfg_path) = config::load_config(&ws, config_path.as_deref())?;

    if cfg.docker_compose_file.is_some() {
        let shell_cmd = vec![
            "sh".to_string(),
            "-c".to_string(),
            "if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi".to_string(),
        ];
        let user = cfg
            .remote_user
            .clone()
            .or_else(|| cfg.container_user.clone());
        let workdir = cfg.workspace_folder.clone();
        return crate::compose::compose_exec(
            &cfg,
            &cfg_path,
            &ws,
            user.as_deref(),
            workdir.as_deref(),
            &shell_cmd,
        );
    }

    let container_name = cfg.container_name(&ws);
    let user = cfg
        .remote_user
        .clone()
        .or_else(|| cfg.container_user.clone());
    let workdir = cfg.workspace_folder.clone();

    let shell_cmd = vec![
        "sh".to_string(),
        "-c".to_string(),
        "if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi".to_string(),
    ];

    let env = crate::command::exec::merged_exec_env(&cfg, &container_name, user.as_deref());
    docker::exec_in_container(
        &container_name,
        user.as_deref(),
        workdir.as_deref(),
        &shell_cmd,
        env.as_ref(),
        Some(&ws),
    )?;

    Ok(())
}
