use std::path::PathBuf;

use crate::config;
use crate::docker;
use crate::error::Result;
use crate::host;

pub fn run(
    workspace_folder: Option<PathBuf>,
    config_path: Option<PathBuf>,
    user: Option<String>,
    workdir: Option<String>,
    command: Vec<String>,
) -> Result<()> {
    docker::check_docker_available()?;

    if let Some(u) = &user
        && u.is_empty()
    {
        eprintln!("Warning: --user is empty; ignoring it");
    }
    if let Some(w) = &workdir
        && w.is_empty()
    {
        eprintln!("Warning: --workdir is empty; ignoring it");
    }

    let ws = docker::get_workspace_folder(workspace_folder)?;
    let (mut cfg, cfg_path) = config::load_config(&ws, config_path.as_deref())?;
    crate::features::apply_cached_feature_container_properties(&mut cfg);

    if cfg.docker_compose_file.is_some() {
        apply_compose_image_metadata(&mut cfg, &cfg_path, &ws);
        let exec_user = user
            .filter(|u| !u.is_empty())
            .or_else(|| cfg.remote_user.clone())
            .or_else(|| cfg.container_user.clone());
        let exec_workdir = workdir
            .filter(|w| !w.is_empty())
            .or(cfg.workspace_folder.clone());
        let env = compose_exec_env(&cfg, &cfg_path, &ws, exec_user.as_deref());
        return crate::compose::compose_exec(
            &cfg,
            &cfg_path,
            &ws,
            exec_user.as_deref(),
            exec_workdir.as_deref(),
            env.as_ref(),
            &command,
        );
    }

    let container_name = cfg.container_name(&ws);
    if docker::container_exists(&container_name)? {
        docker::ensure_container_matches_workspace(&container_name, &ws)?;
        if let Some(raw) = docker::container_metadata_label(&container_name) {
            crate::features::apply_image_metadata(&mut cfg, &raw);
        }
    }
    let exec_user = user
        .filter(|u| !u.is_empty())
        .or_else(|| cfg.remote_user.clone())
        .or_else(|| cfg.container_user.clone());
    let exec_workdir = workdir.filter(|w| !w.is_empty()).or_else(|| {
        Some(if cfg.docker_compose_file.is_some() {
            cfg.workspace_folder
                .clone()
                .unwrap_or_else(|| "/".to_string())
        } else {
            cfg.workspace_folder_or_default(&ws)
        })
    });

    let env = merged_exec_env(&cfg, &container_name, exec_user.as_deref());
    // `${containerWorkspaceFolder}` expansions always use the configured
    // workspace folder, even when `--workdir` overrides the working directory.
    let container_workspace = cfg.workspace_folder_or_default(&ws);
    let container_env_map: std::collections::HashMap<String, String> = cfg
        .container_env
        .iter()
        .map(|(k, v)| {
            let resolved = docker::resolve_container_env_value(v, &cfg.container_env);
            (
                k.clone(),
                docker::expand_vars_for_host_with_target(&resolved, &ws, &container_workspace),
            )
        })
        .collect();
    docker::exec_in_container(
        &container_name,
        exec_user.as_deref(),
        exec_workdir.as_deref(),
        &container_workspace,
        &command,
        env.as_ref(),
        Some(&ws),
        Some(&container_env_map),
    )?;

    Ok(())
}

pub fn merged_exec_env(
    cfg: &config::DevContainerConfig,
    container_name: &str,
    exec_user: Option<&str>,
) -> Option<std::collections::HashMap<String, String>> {
    let mut merged = cfg.remote_env_resolved();
    let probe = host::effective_probe(cfg.user_env_probe.as_deref());
    if !host::is_known_probe(probe) {
        eprintln!("Warning: unknown userEnvProbe '{probe}'");
    } else if probe != "none"
        && let Some(probed) = host::probe_user_env(container_name, exec_user, probe)
    {
        for (k, v) in probed {
            merged.entry(k).or_insert(v);
        }
    }
    if merged.is_empty() {
        None
    } else {
        Some(merged)
    }
}

/// Merge the compose service image's `devcontainer.metadata` into the config.
pub(crate) fn apply_compose_image_metadata(
    cfg: &mut config::DevContainerConfig,
    cfg_path: &std::path::Path,
    ws: &std::path::Path,
) {
    let Ok(name) = crate::compose::get_service_container_name(cfg, cfg_path, ws) else {
        return;
    };
    if let Some(raw) = docker::container_metadata_label(&name) {
        crate::features::apply_image_metadata(cfg, &raw);
    }
}

/// Remote env for compose exec, probing the service container when possible.
pub fn compose_exec_env(
    cfg: &config::DevContainerConfig,
    cfg_path: &std::path::Path,
    ws: &std::path::Path,
    exec_user: Option<&str>,
) -> Option<std::collections::HashMap<String, String>> {
    if let Ok(container_name) = crate::compose::get_service_container_name(cfg, cfg_path, ws) {
        merged_exec_env(cfg, &container_name, exec_user)
    } else if cfg.remote_env_resolved().is_empty() {
        None
    } else {
        Some(cfg.remote_env_resolved())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merged_exec_env_without_probe() {
        let cfg = config::DevContainerConfig {
            remote_env: std::collections::HashMap::from([("A".to_string(), Some("1".to_string()))]),
            ..Default::default()
        };
        let env = merged_exec_env(&cfg, "container", None).unwrap();
        assert_eq!(env.get("A").unwrap(), "1");
        assert_eq!(env.len(), 1);
    }

    #[test]
    fn test_merged_exec_env_empty() {
        let cfg = config::DevContainerConfig::default();
        assert!(merged_exec_env(&cfg, "container", None).is_none());
    }
}
