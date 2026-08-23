use std::path::PathBuf;

use jsonschema::Validator;

use crate::config;
use crate::docker;
use crate::error::Result;

static SCHEMA_JSON: &str = include_str!("../schemas/devcontainer.schema.json");

pub fn run(
    workspace_folder: Option<PathBuf>,
    config_path: Option<PathBuf>,
    include_merged_configuration: bool,
) -> Result<()> {
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
    if let Some(other) = &cfg.other_ports_attributes {
        println!("Other ports attributes: {other}");
    }
    if let Some(probe) = &cfg.user_env_probe {
        println!("userEnvProbe: {probe}");
    }
    if let Some(feat) = &cfg.features {
        println!("Features: {} feature(s)", feat.len());
        for (id, opts) in feat {
            println!("  {id}: {opts}");
        }
    }
    if !cfg.extra.is_empty() {
        println!("Unknown/custom fields (in extra):");
        for (k, v) in &cfg.extra {
            if k == "$schema" {
                continue;
            }
            println!("  {k}: {v}");
        }
    }

    // Strict JSON Schema validation against the official devcontainer schema.
    // Validate the raw file contents (after comment stripping), not the
    // serialized struct which contains all fields including nulls.
    let mut errors: Vec<String> = Vec::new();
    let raw = std::fs::read_to_string(&cfg_path).map_err(crate::error::BondarError::Io)?;
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let stripped = crate::config::strip_json_comments(raw);
    let raw_value: serde_json::Value = serde_json::from_str(&stripped)
        .map_err(|e| crate::error::BondarError::Config(format!("Invalid JSON: {e}")))?;

    let schema: serde_json::Value = serde_json::from_str(SCHEMA_JSON)
        .map_err(|e| crate::error::BondarError::Config(format!("Schema parse error: {e}")))?;
    let validator = Validator::options()
        .with_draft(jsonschema::Draft::Draft201909)
        .build(&schema)
        .map_err(|e| crate::error::BondarError::Config(format!("Schema build error: {e}")))?;

    for e in validator.iter_errors(&raw_value) {
        errors.push(format!("  {}: {}", e.instance_path, e));
    }

    // Additional cross-field validation (deduplicated against schema errors)
    let push_error = |errors: &mut Vec<String>, msg: String| {
        if !errors.contains(&msg) {
            errors.push(msg);
        }
    };
    if cfg.image.is_none() && cfg.build.is_none() && cfg.docker_compose_file.is_none() {
        push_error(
            &mut errors,
            "  No image/build/dockerComposeFile specified".to_string(),
        );
    }
    if cfg.docker_compose_file.is_some() && cfg.service.is_none() {
        push_error(
            &mut errors,
            "  dockerComposeFile requires service".to_string(),
        );
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
            push_error(&mut errors, format!("  waitFor '{wait}' is invalid"));
        }
    }

    if errors.is_empty() {
        println!("\nConfiguration is valid (schema: devContainer.base.schema.json)");
    } else {
        println!("\nConfiguration errors ({}):", errors.len());
        for e in &errors {
            eprintln!("{e}");
        }
        eprintln!("Configuration is INVALID");
        std::process::exit(1);
    }

    if cfg.docker_compose_file.is_some() {
        let file_str = match &cfg.docker_compose_file {
            Some(config::ComposeFileValue::Single(s)) => s.clone(),
            Some(config::ComposeFileValue::Multiple(v)) => v.join(", "),
            None => String::new(),
        };
        println!(
            "Mode: Docker Compose (service: {}, file: {file_str})",
            cfg.service.as_deref().unwrap_or("unknown")
        );
    } else if cfg.build.is_some() {
        let dockerfile = cfg
            .build
            .as_ref()
            .and_then(|b| b.dockerfile.as_deref())
            .unwrap_or("Dockerfile");
        println!("Mode: Build (dockerfile: {dockerfile})");
    } else {
        println!(
            "Mode: Image ({})",
            cfg.image.as_deref().unwrap_or("unknown")
        );
    }

    if include_merged_configuration {
        print_merged_configuration(&cfg, &ws);
    }

    Ok(())
}

fn print_merged_configuration(cfg: &config::DevContainerConfig, ws: &std::path::Path) {
    use serde_json::{Map, Value, json};

    let mut merged: Map<String, Value> = Map::new();

    let mut env: Map<String, Value> = Map::new();
    let target = if cfg.docker_compose_file.is_some() {
        cfg.workspace_folder
            .clone()
            .unwrap_or_else(|| "/".to_string())
    } else {
        cfg.workspace_folder_or_default()
    };
    for (k, v) in cfg.container_env.iter().chain(cfg.remote_env.iter()) {
        let expanded = docker::expand_vars_for_host_with_target(v, ws, &target);
        env.insert(k.clone(), Value::String(expanded));
    }
    for (k, v) in docker::resolve_secrets(cfg) {
        env.insert(k, Value::String(v));
    }

    merged.insert("name".into(), json!(cfg.name));
    merged.insert("image".into(), json!(cfg.image));
    merged.insert("workspaceFolder".into(), json!(cfg.workspace_folder));
    merged.insert("remoteUser".into(), json!(cfg.remote_user));
    merged.insert("containerUser".into(), json!(cfg.container_user));
    merged.insert("mergedEnvironment".into(), Value::Object(env));
    merged.insert(
        "forwardPorts".into(),
        json!(
            cfg.forward_ports
                .iter()
                .map(|p| match p {
                    config::ForwardPort::Number(n) => n.to_string(),
                    config::ForwardPort::Text(s) => s.clone(),
                })
                .collect::<Vec<_>>()
        ),
    );
    merged.insert("mounts".into(), json!(cfg.mounts));
    merged.insert("features".into(), json!(cfg.features));
    merged.insert("runServices".into(), json!(cfg.run_services));
    merged.insert("shutdownAction".into(), json!(cfg.shutdown_action));
    merged.insert("waitFor".into(), json!(cfg.wait_for));
    merged.insert("containerName".into(), json!(cfg.container_name(ws)));
    let default_ws = if cfg.docker_compose_file.is_some() {
        cfg.workspace_folder
            .clone()
            .unwrap_or_else(|| "/".to_string())
    } else {
        cfg.workspace_folder_or_default()
    };
    merged.insert("defaultWorkspaceFolder".into(), json!(default_ws));

    println!("\n--- Merged configuration ---");
    if let Ok(json_str) = serde_json::to_string_pretty(&Value::Object(merged)) {
        println!("{json_str}");
    }
}
