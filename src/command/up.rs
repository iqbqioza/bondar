use std::path::PathBuf;

use crate::config;
use crate::docker;
use crate::error::Result;
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

    if cfg.docker_compose_file.is_some() {
        return Err(crate::error::BondarError::Config(
            "dockerComposeFile is not yet supported. Use image/build.".to_string(),
        ));
    }

    let summary = lifecycle::lifecycle_summary(&cfg);
    if !summary.is_empty() {
        println!("Detected lifecycle: {}", summary.join(", "));
    }

    if cfg.extra.contains_key("features") {
        eprintln!("Warning: 'features' is not yet supported and will be ignored");
    }

    if let Some(action) = &cfg.shutdown_action
        && action != "stopContainer"
    {
        eprintln!(
            "Warning: shutdownAction '{action}' is not fully supported, defaulting to stopContainer"
        );
    }

    if cfg.update_remote_user_uid.is_some() {
        eprintln!("Warning: updateRemoteUserUID is not yet implemented");
    }

    if cfg.host_requirements.is_some() {
        eprintln!("Warning: hostRequirements is not yet enforced");
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
        println!("waitFor: {wait} (handled as sequential execution)");
    }

    println!("Container {container_name} is up");
    println!("  Image: {image_name}");
    println!("  Workspace: {}", ws.display());
    println!("  Use 'bondar exec -- <command>' to execute commands");
    println!("  Use 'bondar shell' for interactive shell");
    println!("  Use 'bondar down' to stop and remove");

    Ok(())
}
