use std::collections::HashMap;
use std::path::Path;

use crate::error::{BondarError, Result};

pub fn handle_features_with_container(
    features: &Option<HashMap<String, serde_json::Value>>,
    override_order: &Option<Vec<String>>,
    container_name: Option<&str>,
    workspace_folder: Option<&Path>,
    container_user: Option<&str>,
) -> Result<()> {
    let Some(feat_map) = features else {
        return Ok(());
    };
    if feat_map.is_empty() {
        return Ok(());
    }

    println!("Features requested: {} feature(s)", feat_map.len());
    for (id, opts) in feat_map {
        println!("  - {id}: {opts}");
    }

    if let Some(order) = override_order {
        println!("Override feature install order: {order:?}");
        let mut missing = Vec::new();
        for id in order {
            if !feat_map.contains_key(id) {
                missing.push(id.clone());
            }
        }
        if !missing.is_empty() {
            eprintln!(
                "Warning: overrideFeatureInstallOrder contains unknown features: {missing:?}"
            );
        }
        println!("Installing features in override order:");
        for id in order {
            if let Some(opts) = feat_map.get(id) {
                install_feature(id, opts, container_name, workspace_folder, container_user)?;
            }
        }
        for (id, opts) in feat_map {
            if !order.contains(id) {
                install_feature(id, opts, container_name, workspace_folder, container_user)?;
            }
        }
    } else {
        let sorted = sort_by_installs_after(feat_map);
        println!("Installing features in installsAfter order:");
        for id in sorted {
            if let Some(opts) = feat_map.get(&id) {
                install_feature(&id, opts, container_name, workspace_folder, container_user)?;
            }
        }
    }

    Ok(())
}

fn sort_by_installs_after(feat_map: &HashMap<String, serde_json::Value>) -> Vec<String> {
    let mut sorted = Vec::new();
    let mut visiting = std::collections::HashSet::new();
    let mut visited = std::collections::HashSet::new();

    fn visit(
        id: &str,
        feat_map: &HashMap<String, serde_json::Value>,
        sorted: &mut Vec<String>,
        visiting: &mut std::collections::HashSet<String>,
        visited: &mut std::collections::HashSet<String>,
    ) {
        if visited.contains(id) {
            return;
        }
        if visiting.contains(id) {
            eprintln!("Warning: circular installsAfter detected for {id}");
            return;
        }
        visiting.insert(id.to_string());
        if let Some(opts) = feat_map.get(id)
            && let Some(arr) = opts.get("installsAfter").and_then(|v| v.as_array())
        {
            for dep in arr {
                if let Some(dep_str) = dep.as_str()
                    && feat_map.contains_key(dep_str)
                {
                    visit(dep_str, feat_map, sorted, visiting, visited);
                }
            }
        }
        visiting.remove(id);
        visited.insert(id.to_string());
        sorted.push(id.to_string());
    }

    for id in feat_map.keys() {
        visit(id, feat_map, &mut sorted, &mut visiting, &mut visited);
    }
    sorted
}

fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

fn run_output(cmd: &mut std::process::Command, desc: &str) -> Result<(bool, String)> {
    let output = cmd
        .output()
        .map_err(|e| BondarError::Docker(format!("Failed to run {desc}: {e}")))?;
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok((output.status.success(), stderr))
}

fn fetch_feature(id: &str, dest_dir: &Path) -> Result<()> {
    let has_oras = std::process::Command::new("oras")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if has_oras {
        println!("  Using 'oras' to fetch OCI artifact");
        let (ok, stderr) = run_output(
            std::process::Command::new("oras").args([
                "pull",
                id,
                "--output",
                dest_dir.to_str().unwrap_or("/tmp"),
            ]),
            &format!("oras pull {id}"),
        )?;
        if ok {
            println!("  Fetched feature {id} via oras");
            return Ok(());
        }
        eprintln!("  Warning: oras pull failed: {stderr}");
        return Err(BondarError::Docker(format!("oras pull failed for {id}")));
    }

    // Fallback: docker pull (works for features that are also container images)
    let feature_image = id.split(':').next().unwrap_or(id);
    println!("  Trying 'docker pull {feature_image}' as fallback");
    let (ok, stderr) = run_output(
        std::process::Command::new("docker").args(["pull", feature_image]),
        "docker pull",
    )?;
    if ok {
        println!("  Pulled feature image {feature_image}");
        Ok(())
    } else {
        eprintln!(
            "  Note: docker pull failed for {feature_image}: {}",
            stderr.lines().next().unwrap_or("")
        );
        Err(BondarError::Docker(format!(
            "Unable to fetch feature {id} (no oras, docker pull failed)"
        )))
    }
}

fn copy_feature_into_container(
    host_dir: &Path,
    container: &str,
    container_path: &str,
) -> Result<()> {
    let (ok, stderr) = run_output(
        std::process::Command::new("docker").args([
            "exec",
            container,
            "sh",
            "-c",
            &format!("mkdir -p {container_path}"),
        ]),
        &format!("mkdir in {container}"),
    )?;
    if !ok {
        eprintln!("  Warning: mkdir failed: {stderr}");
    }

    let (ok, stderr) = run_output(
        std::process::Command::new("docker").args([
            "cp",
            host_dir.to_str().unwrap_or(""),
            &format!("{container}:{container_path}/"),
        ]),
        "docker cp",
    )?;
    if !ok {
        return Err(BondarError::Docker(format!("docker cp failed: {stderr}")));
    }
    Ok(())
}

fn install_in_container(
    id: &str,
    opts: &serde_json::Value,
    container: &str,
    container_path: &str,
    container_user: Option<&str>,
) -> Result<()> {
    let script_path = format!("{container_path}/install.sh");
    let (found, _) = run_output(
        std::process::Command::new("docker").args([
            "exec",
            container,
            "sh",
            "-c",
            &format!("test -f {script_path} && echo yes"),
        ]),
        "check install.sh",
    )?;
    if !found {
        eprintln!("  Warning: install.sh not found at {script_path}, skipping execution");
        return Ok(());
    }

    println!(
        "  Found install.sh, executing inside {container} (as root, per devcontainer spec)..."
    );
    let mut exec_cmd = std::process::Command::new("docker");
    exec_cmd.arg("exec");
    // Feature install scripts must run as root by spec; user info is passed via env
    if let Some(user) = container_user {
        exec_cmd.arg("-e").arg(format!("_CONTAINER_USER={user}"));
        exec_cmd.arg("-e").arg(format!("_REMOTE_USER={user}"));
        exec_cmd.arg("-e").arg(format!("_USERNAME={user}"));
        exec_cmd
            .arg("-e")
            .arg(format!("_CONTAINER_USER_HOME=/home/{user}"));
        exec_cmd
            .arg("-e")
            .arg(format!("_REMOTE_USER_HOME=/home/{user}"));
    }
    exec_cmd.arg(container);
    exec_cmd.arg("sh").arg("-c").arg(format!(
        "cd {container_path} && chmod +x install.sh && ./install.sh"
    ));
    // Pass feature options as environment variables
    if let serde_json::Value::Object(map) = opts {
        for (k, v) in map {
            let value = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => v.to_string(),
            };
            exec_cmd.arg("-e").arg(format!("{k}={value}"));
        }
    }

    let (ok, stderr) = run_output(&mut exec_cmd, "install.sh")?;
    if ok {
        println!("  Feature {id} installed successfully");
    } else {
        eprintln!("  Warning: install.sh failed for {id}: {stderr}");
    }

    // Cleanup the copied files inside the container
    let _ = run_output(
        std::process::Command::new("docker").args([
            "exec",
            container,
            "sh",
            "-c",
            &format!("rm -rf {container_path}"),
        ]),
        "cleanup",
    );

    Ok(())
}

fn read_feature_metadata(dir: &Path) -> Option<serde_json::Value> {
    let path = dir.join("devcontainer-features.json");
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn install_feature(
    id: &str,
    opts: &serde_json::Value,
    container_name: Option<&str>,
    _workspace_folder: Option<&Path>,
    container_user: Option<&str>,
) -> Result<()> {
    if !id.contains('/') && !id.contains('.') {
        eprintln!("Warning: feature ID '{id}' looks invalid, skipping");
        return Ok(());
    }

    let has_docker = std::process::Command::new("docker")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_docker {
        eprintln!("Warning: docker not available, cannot install feature '{id}'");
        return Ok(());
    }

    println!("Attempting to install feature '{id}' with opts {opts}...");

    let dest_dir = std::path::PathBuf::from("/tmp/bondar_features").join(sanitize_id(id));
    if let Err(e) = fetch_feature(id, &dest_dir) {
        eprintln!("  Warning: could not fetch feature {id}: {e}");
        return Ok(());
    }

    // Read devcontainer-features.json metadata for installsAfter dependencies
    if let Some(meta) = read_feature_metadata(&dest_dir)
        && let Some(after) = meta.get("installsAfter").and_then(|v| v.as_array())
    {
        let deps: Vec<String> = after
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        if !deps.is_empty() {
            println!("  Feature declares installsAfter: {deps:?}");
        }
    }

    if let Some(container) = container_name {
        let container_path = format!("/tmp/bondar_features/{}", sanitize_id(id));
        if let Err(e) = copy_feature_into_container(&dest_dir, container, &container_path) {
            eprintln!("  Warning: could not copy feature into container: {e}");
            return Ok(());
        }
        install_in_container(id, opts, container, &container_path, container_user)?;
    } else {
        println!(
            "  Feature {id} fetched to {}. Execution requires a running container (use 'bondar up' first).",
            dest_dir.display()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_handle_empty() {
        assert!(handle_features_with_container(&None, &None, None, None, None).is_ok());
        let empty: HashMap<String, serde_json::Value> = HashMap::new();
        assert!(handle_features_with_container(&Some(empty), &None, None, None, None).is_ok());
    }

    #[test]
    fn test_sanitize_id() {
        assert_eq!(
            sanitize_id("ghcr.io/devcontainers/features/common-utils:2"),
            "ghcr-io-devcontainers-features-common-utils-2"
        );
    }

    #[test]
    fn test_sort_by_installs_after_orders_dependencies() {
        let mut feat_map: HashMap<String, serde_json::Value> = HashMap::new();
        feat_map.insert(
            "ghcr.io/a/child".to_string(),
            serde_json::json!({ "installsAfter": ["ghcr.io/a/base"] }),
        );
        feat_map.insert("ghcr.io/a/base".to_string(), serde_json::json!({}));
        let sorted = sort_by_installs_after(&feat_map);
        let base_pos = sorted.iter().position(|x| x == "ghcr.io/a/base").unwrap();
        let child_pos = sorted.iter().position(|x| x == "ghcr.io/a/child").unwrap();
        assert!(base_pos < child_pos);
    }

    #[test]
    fn test_sort_by_installs_after_no_circular_hang() {
        let mut feat_map: HashMap<String, serde_json::Value> = HashMap::new();
        feat_map.insert(
            "ghcr.io/a/x".to_string(),
            serde_json::json!({ "installsAfter": ["ghcr.io/a/y"] }),
        );
        feat_map.insert(
            "ghcr.io/a/y".to_string(),
            serde_json::json!({ "installsAfter": ["ghcr.io/a/x"] }),
        );
        let sorted = sort_by_installs_after(&feat_map);
        assert_eq!(sorted.len(), 2);
    }
}
