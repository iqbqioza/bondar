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
) -> Result<()> {
    docker::check_docker_available()?;

    let ws = docker::get_workspace_folder(workspace_folder)?;
    let (cfg, cfg_path) = config::load_config(&ws, config_path.as_deref())?;

    let summary = lifecycle::lifecycle_summary(&cfg);
    if !summary.is_empty() {
        println!("Detected lifecycle: {}", summary.join(", "));
    }

    if cfg.extra.contains_key("features") {
        eprintln!("Warning: 'features' is not yet supported and will be ignored");
    }
    if let Some(order) = &cfg.override_feature_install_order {
        eprintln!("Warning: overrideFeatureInstallOrder {order:?} is not yet supported");
    }
    if let Some(probe) = &cfg.user_env_probe {
        eprintln!("Warning: userEnvProbe '{probe}' is not yet implemented");
    }
    if cfg.ports_attributes.is_some() || cfg.other_ports_attributes.is_some() {
        eprintln!("Warning: portsAttributes/otherPortsAttributes are for UI only, ignored");
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
        return run_compose(&cfg, &cfg_path, &ws, remove_existing);
    }

    let container_name = cfg.container_name(&ws);
    let was_existing = docker::container_exists(&container_name)?;
    let was_running = if was_existing {
        docker::container_running(&container_name)?
    } else {
        false
    };

    if let Some(cmd) = &cfg.initialize_command {
        println!("Running initializeCommand on host...");
        lifecycle::execute_host_lifecycle(cmd, &ws)?;
    }

    let image_name = docker::resolve_image_name(&cfg, &cfg_path, &ws)?;

    if cfg.build.is_some() && !no_build {
        docker::build_image(&cfg, &cfg_path, &ws, &image_name, false)?;
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

    let workspace_target = cfg.workspace_folder_or_default();
    let exec_user = cfg.remote_user.as_deref().or(cfg.container_user.as_deref());
    let newly_created = !was_existing;

    if newly_created {
        if let Some(cmd) = &cfg.on_create_command {
            println!("Running onCreateCommand...");
            lifecycle::execute_container_lifecycle(
                cmd,
                &container_name,
                exec_user,
                &workspace_target,
                &ws,
            )?;
        }
        if let Some(cmd) = &cfg.update_content_command {
            println!("Running updateContentCommand...");
            lifecycle::execute_container_lifecycle(
                cmd,
                &container_name,
                exec_user,
                &workspace_target,
                &ws,
            )?;
        }
        if let Some(cmd) = &cfg.post_create_command {
            println!("Running postCreateCommand...");
            lifecycle::execute_container_lifecycle(
                cmd,
                &container_name,
                exec_user,
                &workspace_target,
                &ws,
            )?;
        }
    }

    let should_run_post_start = newly_created || !was_running;
    if should_run_post_start && let Some(cmd) = &cfg.post_start_command {
        println!("Running postStartCommand...");
        lifecycle::execute_container_lifecycle(
            cmd,
            &container_name,
            exec_user,
            &workspace_target,
            &ws,
        )?;
    }

    if let Some(cmd) = &cfg.post_attach_command {
        println!("Running postAttachCommand...");
        lifecycle::execute_container_lifecycle(
            cmd,
            &container_name,
            exec_user,
            &workspace_target,
            &ws,
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
        } else {
            println!("waitFor: {wait} (handled as sequential execution)");
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

fn run_compose(
    cfg: &config::DevContainerConfig,
    cfg_path: &std::path::Path,
    ws: &std::path::Path,
    _remove_existing: bool,
) -> Result<()> {
    crate::compose::check_compose_available()?;

    if let Some(cmd) = &cfg.initialize_command {
        println!("Running initializeCommand on host...");
        lifecycle::execute_host_lifecycle(cmd, ws)?;
    }

    crate::compose::compose_up(cfg, cfg_path, ws)?;

    let service = cfg.service.as_deref().unwrap_or("service");
    let container_name = crate::compose::get_service_container_name(cfg, cfg_path, ws)
        .unwrap_or_else(|_| service.to_string());

    host::handle_update_remote_user_uid(cfg, &container_name).ok();

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
