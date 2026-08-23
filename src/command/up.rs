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

    let ws = docker::get_workspace_folder(workspace_folder)?;
    let (cfg, cfg_path) = config::load_config(&ws, config_path.as_deref())?;

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
        return run_compose(&cfg, &cfg_path, &ws, remove_existing, no_cache);
    }

    let container_name = cfg.container_name(&ws);
    let was_existing = docker::container_exists(&container_name)?;
    let was_running = if was_existing {
        docker::container_running(&container_name)?
    } else {
        false
    };

    // Warn if another container exists for the same workspace (name collision)
    if let Ok(others) = docker::find_containers_for_workspace(&ws) {
        for other in others {
            if other != container_name {
                eprintln!(
                    "Warning: existing container '{other}' is bound to this workspace; consider '--remove-existing-container' or a unique 'name'"
                );
            }
        }
    }

    if let Some(cmd) = &cfg.initialize_command {
        println!("Running initializeCommand on host...");
        lifecycle::execute_host_lifecycle(cmd, &ws)?;
    }

    let image_name = docker::resolve_image_name(&cfg, &cfg_path, &ws)?;

    if cfg.build.is_some() && !no_build {
        docker::build_image(&cfg, &cfg_path, &ws, &image_name, no_cache)?;
    }

    docker::create_and_start_container(
        &cfg,
        &ws,
        &cfg_path,
        &container_name,
        &image_name,
        remove_existing,
    )?;

    host::handle_update_remote_user_uid(&cfg, &container_name)?;

    crate::features::handle_features_with_container(
        &cfg.features,
        &cfg.override_feature_install_order,
        Some(&container_name),
        Some(&ws),
        cfg.remote_user.as_deref(),
    )?;

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
    let newly_created = !was_existing;
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
                0,
                wait_idx,
                cmd,
                &container_name,
                exec_user,
                &workspace_target,
                &ws,
                lifecycle_env.as_ref(),
            )?;
        }
        if let Some(cmd) = &cfg.update_content_command {
            run_lifecycle_step(
                "updateContentCommand",
                1,
                wait_idx,
                cmd,
                &container_name,
                exec_user,
                &workspace_target,
                &ws,
                lifecycle_env.as_ref(),
            )?;
        }
        if let Some(cmd) = &cfg.post_create_command {
            run_lifecycle_step(
                "postCreateCommand",
                2,
                wait_idx,
                cmd,
                &container_name,
                exec_user,
                &workspace_target,
                &ws,
                lifecycle_env.as_ref(),
            )?;
        }
    }

    let should_run_post_start = newly_created || !was_running;
    if should_run_post_start && let Some(cmd) = &cfg.post_start_command {
        run_lifecycle_step(
            "postStartCommand",
            3,
            wait_idx,
            cmd,
            &container_name,
            exec_user,
            &workspace_target,
            &ws,
            lifecycle_env.as_ref(),
        )?;
    }

    if let Some(cmd) = &cfg.post_attach_command {
        run_lifecycle_step(
            "postAttachCommand",
            4,
            wait_idx,
            cmd,
            &container_name,
            exec_user,
            &workspace_target,
            &ws,
            lifecycle_env.as_ref(),
        )?;
    }

    if let Some(wait) = &cfg.wait_for {
        let valid = [
            "initializeCommand",
            "onCreateCommand",
            "updateContentCommand",
            "postCreateCommand",
            "postStartCommand",
            "postAttachCommand",
        ];
        if !valid.contains(&wait.as_str()) {
            eprintln!("Warning: waitFor '{wait}' is not a valid lifecycle command");
        } else if wait_idx < 5 {
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
        "onCreateCommand",
        "updateContentCommand",
        "postCreateCommand",
        "postStartCommand",
        "postAttachCommand",
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
        )
    }
}

fn run_compose(
    cfg: &config::DevContainerConfig,
    cfg_path: &std::path::Path,
    ws: &std::path::Path,
    remove_existing: bool,
    no_cache: bool,
) -> Result<()> {
    crate::compose::check_compose_available()?;

    if let Some(cmd) = &cfg.initialize_command {
        println!("Running initializeCommand on host...");
        lifecycle::execute_host_lifecycle(cmd, ws)?;
    }

    if no_cache && cfg.docker_compose_file.is_some() {
        // Force rebuild with no cache for compose
        let mut build_cmd = std::process::Command::new("docker");
        build_cmd.arg("compose");
        for arg in crate::compose::compose_files_args_for_build(cfg, cfg_path)? {
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
    }

    crate::compose::compose_up(cfg, cfg_path, ws, remove_existing)?;

    let service = cfg.service.as_deref().unwrap_or("service");
    let container_name = crate::compose::get_service_container_name(cfg, cfg_path, ws)
        .unwrap_or_else(|_| service.to_string());

    host::handle_update_remote_user_uid(cfg, &container_name).ok();
    crate::features::handle_features_with_container(
        &cfg.features,
        &cfg.override_feature_install_order,
        Some(&container_name),
        Some(ws),
        cfg.remote_user.as_deref(),
    )
    .ok();

    let workspace_target = cfg
        .workspace_folder
        .clone()
        .unwrap_or_else(|| "/".to_string());
    let exec_user = cfg.remote_user.as_deref().or(cfg.container_user.as_deref());

    if let Some(cmd) = &cfg.on_create_command {
        println!("Running onCreateCommand in compose service...");
        lifecycle::execute_container_lifecycle(
            cmd,
            &container_name,
            exec_user,
            &workspace_target,
            ws,
        )
        .ok();
    }
    if let Some(cmd) = &cfg.update_content_command {
        lifecycle::execute_container_lifecycle(
            cmd,
            &container_name,
            exec_user,
            &workspace_target,
            ws,
        )
        .ok();
    }
    if let Some(cmd) = &cfg.post_create_command {
        println!("Running postCreateCommand in compose service...");
        lifecycle::execute_container_lifecycle(
            cmd,
            &container_name,
            exec_user,
            &workspace_target,
            ws,
        )
        .ok();
    }
    if let Some(cmd) = &cfg.post_start_command {
        println!("Running postStartCommand in compose service...");
        lifecycle::execute_container_lifecycle(
            cmd,
            &container_name,
            exec_user,
            &workspace_target,
            ws,
        )
        .ok();
    }
    if let Some(cmd) = &cfg.post_attach_command {
        println!("Running postAttachCommand in compose service...");
        lifecycle::execute_container_lifecycle(
            cmd,
            &container_name,
            exec_user,
            &workspace_target,
            ws,
        )
        .ok();
    }

    println!("Compose service {service} is up (container {container_name})");
    println!("  Workspace: {}", ws.display());
    Ok(())
}
