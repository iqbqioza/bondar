use std::path::PathBuf;

use crate::config;
use crate::docker;
use crate::error::Result;
use crate::host;
use crate::lifecycle;

pub fn run(
    workspace_folder: Option<PathBuf>,
    config_path: Option<PathBuf>,
    remove_existing: bool,
    no_build: bool,
    no_cache: bool,
) -> Result<()> {
    docker::check_docker_available()?;

    if no_build && no_cache {
        eprintln!(
            "Warning: --no-build and --no-cache combined; build is skipped so --no-cache has no effect"
        );
    }

    let ws = docker::get_workspace_folder(workspace_folder)?;
    let (cfg, cfg_path) = config::load_config(&ws, config_path.as_deref())?;

    if no_build && cfg.build.is_none() && cfg.docker_compose_file.is_none() {
        eprintln!("Warning: --no-build has no effect (no 'build' section configured)");
    }
    if no_cache && cfg.build.is_none() && cfg.docker_compose_file.is_none() {
        eprintln!("Warning: --no-cache has no effect (no 'build' section configured)");
    }

    let summary = lifecycle::lifecycle_summary(&cfg);
    if !summary.is_empty() {
        println!("Detected lifecycle: {}", summary.join(", "));
    }

    if let Some(probe) = &cfg.user_env_probe {
        match probe.as_str() {
            "none" => {}
            "interactiveShell" | "loginShell" | "loginInteractiveShell" => {
                println!("userEnvProbe: {probe} (will probe via shell)");
            }
            _ => eprintln!("Warning: unknown userEnvProbe '{probe}'"),
        }
    }

    if let Some(attrs) = &cfg.ports_attributes {
        println!("portsAttributes: {attrs} (stored as container labels)");
    }
    if let Some(other) = &cfg.other_ports_attributes {
        println!("otherPortsAttributes: {other}");
    }
    if let Some(action) = &cfg.shutdown_action
        && action != "stopContainer"
        && action != "stopCompose"
        && action != "none"
    {
        eprintln!("Warning: unknown shutdownAction '{action}'");
    }
    if let Some(req) = &cfg.host_requirements {
        host::check_host_requirements(req, &ws)?;
    }

    if cfg.docker_compose_file.is_some() {
        return run_compose(&cfg, &cfg_path, &ws, remove_existing, no_build, no_cache);
    }

    let container_name = cfg.container_name(&ws);
    if container_name.len() > 255 {
        eprintln!(
            "Warning: container name '{container_name}' exceeds Docker's 255 character limit; 'docker run' may fail"
        );
    }
    let (was_existing, was_running) = docker::container_exists_and_running(&container_name)?;

    // Warn if another container exists for the same workspace (name collision)
    match docker::find_containers_for_workspace(&ws) {
        Ok(others) => {
            for other in others {
                if other != container_name {
                    eprintln!(
                        "Warning: existing container '{other}' is bound to this workspace; consider '--remove-existing-container' or a unique 'name'"
                    );
                }
            }
        }
        Err(e) => {
            eprintln!("Warning: could not check for existing containers: {e}");
        }
    }

    if let Some(cmd) = &cfg.initialize_command {
        println!("Running initializeCommand on host...");
        lifecycle::execute_host_lifecycle(cmd, &ws)?;
    }

    let image_name = docker::resolve_image_name(&cfg, &ws)?;

    if cfg.build.is_some() && !no_build {
        docker::build_image(&cfg, &cfg_path, &ws, &image_name, no_cache)?;
    } else if cfg.build.is_some() {
        println!("Skipping image build (--no-build)");
    }

    docker::create_and_start_container(
        &cfg,
        &ws,
        &cfg_path,
        &container_name,
        &image_name,
        remove_existing,
    )?;

    // A fresh container exists when there was none before, or when
    // --remove-existing-container forced a recreate.
    let newly_created = !was_existing || remove_existing;

    host::handle_update_remote_user_uid(&cfg, &container_name)?;

    // Features install once at container creation; do not re-install on
    // restart of an existing container.
    if newly_created {
        crate::features::handle_features_with_container(
            &cfg.features,
            &cfg.override_feature_install_order,
            Some(&container_name),
            cfg.remote_user.as_deref(),
        )?;

        // Store merged feature customizations as a container label
        let merged_custom = crate::features::collect_feature_customizations(&cfg.features);
        if !merged_custom.as_object().is_none_or(|m| m.is_empty()) {
            let json_str = serde_json::to_string(&merged_custom).unwrap_or_default();
            let label_arg = format!("devcontainer.feature_customizations={json_str}");
            // `docker update --label-add` is supported broadly; fall back to
            // `docker label` (Docker 25+) for stopped containers.
            let ok = std::process::Command::new("docker")
                .args(["update", "--label-add", &label_arg, &container_name])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                let _ = std::process::Command::new("docker")
                    .args(["label", &container_name, &label_arg])
                    .status();
            }
            println!("Merged feature customizations stored on container label");
        }
    }

    let probed_env = if let Some(probe) = &cfg.user_env_probe
        && probe != "none"
    {
        println!("Probing user env with {probe}...");
        let exec_user = cfg.remote_user.as_deref().or(cfg.container_user.as_deref());
        host::probe_user_env(&container_name, exec_user, probe)
    } else {
        None
    };
    if let Some(env) = &probed_env {
        println!("Probed {} env vars", env.len());
    }

    let workspace_target = cfg.workspace_folder_or_default();
    let exec_user = cfg.remote_user.as_deref().or(cfg.container_user.as_deref());
    let container_env_map: std::collections::HashMap<String, String> = cfg
        .container_env
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                crate::docker::expand_vars_for_host_with_target(v, &ws, &workspace_target),
            )
        })
        .collect();
    let lifecycle_env = {
        let mut merged = cfg.remote_env.clone();
        if let Some(probed) = &probed_env {
            for (k, v) in probed {
                merged.entry(k.clone()).or_insert(v.clone());
            }
        }
        if merged.is_empty() {
            None
        } else {
            Some(merged)
        }
    };

    let wait_idx = wait_index(&cfg.wait_for);

    if newly_created {
        if let Some(cmd) = &cfg.on_create_command {
            run_lifecycle_step(
                "onCreateCommand",
                1,
                wait_idx,
                cmd,
                &container_name,
                exec_user,
                &workspace_target,
                &ws,
                lifecycle_env.as_ref(),
                &container_env_map,
            )?;
        }
        if let Some(cmd) = &cfg.update_content_command {
            run_lifecycle_step(
                "updateContentCommand",
                2,
                wait_idx,
                cmd,
                &container_name,
                exec_user,
                &workspace_target,
                &ws,
                lifecycle_env.as_ref(),
                &container_env_map,
            )?;
        }
        if let Some(cmd) = &cfg.post_create_command {
            run_lifecycle_step(
                "postCreateCommand",
                3,
                wait_idx,
                cmd,
                &container_name,
                exec_user,
                &workspace_target,
                &ws,
                lifecycle_env.as_ref(),
                &container_env_map,
            )?;
        }
    }

    let should_run_post_start = newly_created || !was_running;
    if should_run_post_start && let Some(cmd) = &cfg.post_start_command {
        run_lifecycle_step(
            "postStartCommand",
            4,
            wait_idx,
            cmd,
            &container_name,
            exec_user,
            &workspace_target,
            &ws,
            lifecycle_env.as_ref(),
            &container_env_map,
        )?;
    }

    if let Some(cmd) = &cfg.post_attach_command {
        run_lifecycle_step(
            "postAttachCommand",
            5,
            wait_idx,
            cmd,
            &container_name,
            exec_user,
            &workspace_target,
            &ws,
            lifecycle_env.as_ref(),
            &container_env_map,
        )?;
    }

    if let Some(wait) = &cfg.wait_for {
        let valid = [
            "initializeCommand",
            "onCreateCommand",
            "updateContentCommand",
            "postCreateCommand",
            "postStartCommand",
        ];
        if !valid.contains(&wait.as_str()) {
            eprintln!("Warning: waitFor '{wait}' is not a valid lifecycle command");
        } else if wait_idx != usize::MAX {
            println!("waitFor: {wait} - commands after this run in background");
        }
    }

    println!("Container {container_name} is up");
    println!("  Image: {image_name}");
    println!("  Workspace: {}", ws.display());
    println!("  Use 'bondar exec -- <command>' to execute commands");
    println!("  Use 'bondar shell' for interactive shell");
    println!("  Use 'bondar down' to stop and remove");

    Ok(())
}

fn wait_index(wait: &Option<String>) -> usize {
    const ORDER: [&str; 5] = [
        "initializeCommand",
        "onCreateCommand",
        "updateContentCommand",
        "postCreateCommand",
        "postStartCommand",
    ];
    wait.as_ref()
        .and_then(|w| ORDER.iter().position(|&x| x == w))
        .unwrap_or(usize::MAX)
}

#[allow(clippy::too_many_arguments)]
fn run_lifecycle_step(
    name: &str,
    step_idx: usize,
    wait_idx: usize,
    cmd: &serde_json::Value,
    container_name: &str,
    exec_user: Option<&str>,
    workspace_target: &str,
    ws: &std::path::Path,
    lifecycle_env: Option<&std::collections::HashMap<String, String>>,
    container_env_map: &std::collections::HashMap<String, String>,
) -> Result<()> {
    if step_idx > wait_idx {
        println!("Running {name} in background (waitFor)...");
        lifecycle::spawn_container_lifecycle(
            cmd,
            container_name,
            exec_user,
            workspace_target,
            ws,
            lifecycle_env,
            Some(container_env_map),
        )
    } else {
        println!("Running {name}...");
        lifecycle::execute_container_lifecycle_with_env(
            cmd,
            container_name,
            exec_user,
            workspace_target,
            ws,
            lifecycle_env,
            Some(container_env_map),
        )
    }
}

fn run_compose(
    cfg: &config::DevContainerConfig,
    cfg_path: &std::path::Path,
    ws: &std::path::Path,
    remove_existing: bool,
    no_build: bool,
    no_cache: bool,
) -> Result<()> {
    crate::compose::check_compose_available()?;

    if no_build && no_cache {
        eprintln!(
            "Warning: --no-build and --no-cache combined; build is skipped so --no-cache has no effect"
        );
    }

    let (was_existing, was_running) = crate::compose::service_container_state(cfg, cfg_path, ws)?;

    if let Some(cmd) = &cfg.initialize_command {
        println!("Running initializeCommand on host...");
        lifecycle::execute_host_lifecycle(cmd, ws)?;
    }

    if no_cache && !no_build {
        // Force rebuild with no cache for compose
        let mut build_cmd = std::process::Command::new("docker");
        build_cmd.arg("compose");
        for arg in crate::compose::compose_files_args_for_build(cfg, cfg_path, ws)? {
            build_cmd.arg(arg);
        }
        build_cmd.arg("build").arg("--no-cache");
        build_cmd.current_dir(ws);
        let status = build_cmd.status().map_err(|e| {
            crate::error::BondarError::Docker(format!("Failed to run compose build: {e}"))
        })?;
        if !status.success() {
            return Err(crate::error::BondarError::Docker(
                "docker compose build failed".to_string(),
            ));
        }
    } else if no_build {
        println!("Skipping compose build (--no-build)");
    }

    crate::compose::compose_up(cfg, cfg_path, ws, remove_existing, no_build)?;

    let newly_created = !was_existing || remove_existing;

    let service = cfg.service.as_deref().unwrap_or("service");
    let container_name = match crate::compose::get_service_container_name(cfg, cfg_path, ws) {
        Ok(name) => name,
        Err(e) => {
            eprintln!(
                "Warning: could not resolve service container name ({e}); falling back to service '{service}'"
            );
            service.to_string()
        }
    };

    host::handle_update_remote_user_uid(cfg, &container_name)?;

    if newly_created {
        crate::features::handle_features_with_container(
            &cfg.features,
            &cfg.override_feature_install_order,
            Some(&container_name),
            cfg.remote_user.as_deref(),
        )?;
    }

    let workspace_target = cfg
        .workspace_folder
        .clone()
        .unwrap_or_else(|| "/".to_string());
    let exec_user = cfg.remote_user.as_deref().or(cfg.container_user.as_deref());
    let container_env_map: std::collections::HashMap<String, String> = cfg
        .container_env
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                crate::docker::expand_vars_for_host_with_target(v, ws, &workspace_target),
            )
        })
        .collect();

    let probed_env = if let Some(probe) = &cfg.user_env_probe
        && probe != "none"
    {
        println!("Probing user env with {probe}...");
        host::probe_user_env(&container_name, exec_user, probe)
    } else {
        None
    };
    let lifecycle_env = {
        let mut merged = cfg.remote_env.clone();
        if let Some(probed) = &probed_env {
            for (k, v) in probed {
                merged.entry(k.clone()).or_insert(v.clone());
            }
        }
        if merged.is_empty() {
            None
        } else {
            Some(merged)
        }
    };

    let wait_idx = wait_index(&cfg.wait_for);

    if newly_created {
        if let Some(cmd) = &cfg.on_create_command {
            run_lifecycle_step(
                "onCreateCommand",
                1,
                wait_idx,
                cmd,
                &container_name,
                exec_user,
                &workspace_target,
                ws,
                lifecycle_env.as_ref(),
                &container_env_map,
            )?;
        }
        if let Some(cmd) = &cfg.update_content_command {
            run_lifecycle_step(
                "updateContentCommand",
                2,
                wait_idx,
                cmd,
                &container_name,
                exec_user,
                &workspace_target,
                ws,
                lifecycle_env.as_ref(),
                &container_env_map,
            )?;
        }
        if let Some(cmd) = &cfg.post_create_command {
            run_lifecycle_step(
                "postCreateCommand",
                3,
                wait_idx,
                cmd,
                &container_name,
                exec_user,
                &workspace_target,
                ws,
                lifecycle_env.as_ref(),
                &container_env_map,
            )?;
        }
    }

    if (newly_created || !was_running)
        && let Some(cmd) = &cfg.post_start_command
    {
        run_lifecycle_step(
            "postStartCommand",
            4,
            wait_idx,
            cmd,
            &container_name,
            exec_user,
            &workspace_target,
            ws,
            lifecycle_env.as_ref(),
            &container_env_map,
        )?;
    }

    if let Some(cmd) = &cfg.post_attach_command {
        run_lifecycle_step(
            "postAttachCommand",
            5,
            wait_idx,
            cmd,
            &container_name,
            exec_user,
            &workspace_target,
            ws,
            lifecycle_env.as_ref(),
            &container_env_map,
        )?;
    }

    if let Some(wait) = &cfg.wait_for {
        let valid = [
            "initializeCommand",
            "onCreateCommand",
            "updateContentCommand",
            "postCreateCommand",
            "postStartCommand",
        ];
        if !valid.contains(&wait.as_str()) {
            eprintln!("Warning: waitFor '{wait}' is not a valid lifecycle command");
        } else if wait_idx != usize::MAX {
            println!("waitFor: {wait} - commands after this run in background");
        }
    }

    println!("Compose service {service} is up (container {container_name})");
    println!("  Workspace: {}", ws.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wait_index_valid() {
        assert_eq!(wait_index(&Some("initializeCommand".to_string())), 0);
        assert_eq!(wait_index(&Some("onCreateCommand".to_string())), 1);
        assert_eq!(wait_index(&Some("postCreateCommand".to_string())), 3);
        assert_eq!(wait_index(&Some("postStartCommand".to_string())), 4);
    }

    #[test]
    fn test_wait_index_invalid_or_missing() {
        assert_eq!(
            wait_index(&Some("postAttachCommand".to_string())),
            usize::MAX
        );
        assert_eq!(wait_index(&Some("bogus".to_string())), usize::MAX);
        assert_eq!(wait_index(&None), usize::MAX);
        assert_eq!(wait_index(&Some(String::new())), usize::MAX);
    }
}
