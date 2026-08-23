use std::path::PathBuf;

use jsonschema::Validator;

use crate::config;
use crate::docker;
use crate::error::{BondarError, Result};

static SCHEMA_JSON: &str = include_str!("../schemas/devcontainer.schema.json");

pub fn run(
    workspace_folder: Option<PathBuf>,
    config_path: Option<PathBuf>,
    include_merged_configuration: bool,
) -> Result<()> {
    let ws = docker::get_workspace_folder(workspace_folder)?;

    // Determine the config path without validating, so read-configuration can
    // report every problem instead of failing on the first one.
    let cfg_path = if let Some(p) = &config_path {
        p.canonicalize().map_err(|e| {
            BondarError::NotFound(format!("Cannot resolve config path {}: {e}", p.display()))
        })?
    } else if let Some(p) = config::find_config_path(&ws) {
        p
    } else {
        return Err(BondarError::NotFound(format!(
            "devcontainer.json not found in {}",
            ws.display()
        )));
    };

    println!("Config file: {}", cfg_path.display());
    println!("Workspace: {}", ws.display());
    println!();

    let raw = std::fs::read_to_string(&cfg_path).map_err(BondarError::Io)?;
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let stripped = config::strip_json_comments(raw);
    let raw_value: serde_json::Value = serde_json::from_str(&stripped)
        .map_err(|e| BondarError::Config(format!("Invalid JSON: {e}")))?;

    let mut errors: Vec<String> = Vec::new();

    // Strict JSON Schema validation against the official devcontainer schema.
    let schema: serde_json::Value = serde_json::from_str(SCHEMA_JSON)
        .map_err(|e| BondarError::Config(format!("Schema parse error: {e}")))?;
    let validator = Validator::options()
        .with_draft(jsonschema::Draft::Draft201909)
        .build(&schema)
        .map_err(|e| BondarError::Config(format!("Schema build error: {e}")))?;
    for e in validator.iter_errors(&raw_value) {
        errors.push(format!("  {}: {}", e.instance_path, e));
    }

    // Typed parse; type errors are reported as validation errors
    let mut typed_ok = true;
    let cfg: config::DevContainerConfig = match serde_json::from_str(&stripped) {
        Ok(c) => c,
        Err(e) => {
            errors.push(format!("  {e}"));
            typed_ok = false;
            config::DevContainerConfig::default()
        }
    };

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

    // Additional cross-field validation (deduplicated against schema errors)
    let push_error = |errors: &mut Vec<String>, msg: String| {
        if !errors.contains(&msg) {
            errors.push(msg);
        }
    };
    // Include the internal validation checks (e.g. absolute workspaceFolder,
    // env keys without '=', non-empty mounts) as reported errors. When the
    // typed parse failed, the default config would add unrelated errors, so
    // those checks are skipped.
    if typed_ok && let Err(e) = cfg.validate() {
        let msg = e.to_string().replace("Config error: ", "");
        push_error(&mut errors, format!("  {msg}"));
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
            // Skip when the schema already reported this (message differs)
            if !errors.iter().any(|e| e.contains("waitFor")) {
                push_error(&mut errors, format!("  waitFor '{wait}' is invalid"));
            }
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
        crate::lifecycle::reap_children();
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
    // Container env and remote env (null remoteEnv entries are skipped)
    let all_env: Vec<(&String, Option<&str>)> = cfg
        .container_env
        .iter()
        .map(|(k, v)| (k, Some(v.as_str())))
        .chain(cfg.remote_env.iter().map(|(k, v)| (k, v.as_deref())))
        .collect();
    for (k, v) in &all_env {
        let Some(val) = v else {
            continue;
        };
        let expanded = docker::expand_vars_for_host_with_target(val, ws, &target);
        env.insert((*k).clone(), Value::String(expanded));
    }
    // Resolve ${containerEnv:KEY} references from the original values against
    // the containerEnv entries (same approach as docker run)
    let container_env_map: std::collections::HashMap<String, String> = cfg
        .container_env
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                docker::expand_vars_for_host_with_target(v, ws, &target),
            )
        })
        .collect();
    for (k, v) in &all_env {
        let Some(val) = v else {
            continue;
        };
        let skip = if cfg.container_env.contains_key(*k) {
            Some((*k).as_str())
        } else {
            None
        };
        let from_map = docker::expand_container_env_from_map(val, &container_env_map, skip);
        let resolved = docker::expand_vars_for_host_with_target(&from_map, ws, &target);
        env.insert((*k).clone(), Value::String(resolved));
    }
    // Secret values are never printed; only their key names are listed.
    let secret_names: Vec<String> = cfg
        .secrets
        .as_ref()
        .map(|s| s.keys().cloned().collect())
        .unwrap_or_default();

    let default_name = ws
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "workspace".to_string());
    merged.insert(
        "name".into(),
        json!(cfg.name.clone().unwrap_or(default_name)),
    );
    merged.insert("image".into(), json!(cfg.image));
    merged.insert("workspaceFolder".into(), json!(cfg.workspace_folder));
    merged.insert("remoteUser".into(), json!(cfg.remote_user));
    merged.insert("containerUser".into(), json!(cfg.container_user));
    merged.insert("mergedEnvironment".into(), Value::Object(env));
    merged.insert("secrets".into(), json!(secret_names));
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
    let expanded_mounts: Vec<Value> = cfg
        .mounts
        .iter()
        .map(|m| match m {
            config::MountValue::String(s) => {
                Value::String(docker::expand_vars_for_host_with_target(s, ws, &target))
            }
            config::MountValue::Object(o) => {
                let mut obj = serde_json::Map::new();
                if let Some(s) = &o.source {
                    obj.insert(
                        "source".into(),
                        Value::String(docker::expand_vars_for_host_with_target(s, ws, &target)),
                    );
                }
                if let Some(t) = &o.target {
                    obj.insert(
                        "target".into(),
                        Value::String(docker::expand_vars_for_host_with_target(t, ws, &target)),
                    );
                }
                if let Some(t) = &o.mount_type {
                    obj.insert("type".into(), Value::String(t.clone()));
                }
                if let Some(r) = o.readonly {
                    obj.insert("readonly".into(), Value::Bool(r));
                }
                Value::Object(obj)
            }
        })
        .collect();
    merged.insert("mounts".into(), Value::Array(expanded_mounts));
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
