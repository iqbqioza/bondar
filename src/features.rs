use std::collections::HashMap;
use std::path::Path;

use crate::error::{BondarError, Result};

/// Collect `customizations` from all fetched feature metadata files and merge
/// them into a single object. Returns an empty map when nothing is available.
pub fn collect_feature_customizations(
    features: &Option<HashMap<String, serde_json::Value>>,
) -> serde_json::Value {
    let Some(feat_map) = features else {
        return serde_json::Value::Object(Default::default());
    };
    let mut merged = serde_json::Map::new();
    for id in feat_map.keys() {
        let dir = feature_cache_dir().join(sanitize_id(id));
        let Some(meta) = read_feature_metadata(&dir) else {
            continue;
        };
        let Some(custom) = meta.get("customizations") else {
            continue;
        };
        let Some(obj) = custom.as_object() else {
            continue;
        };
        for (tool, value) in obj {
            let entry = merged
                .entry(tool.clone())
                .or_insert_with(|| serde_json::Value::Object(Default::default()));
            if let Some(existing) = entry.as_object_mut()
                && let Some(incoming) = value.as_object()
            {
                for (k, v) in incoming {
                    existing.insert(k.clone(), v.clone());
                }
            }
        }
    }
    serde_json::Value::Object(merged)
}

pub fn handle_features_with_container(
    features: &Option<HashMap<String, serde_json::Value>>,
    override_order: &Option<Vec<String>>,
    container_name: Option<&str>,
    container_user: Option<&str>,
) -> Result<()> {
    let Some(feat_map) = features else {
        return Ok(());
    };
    if feat_map.is_empty() {
        return Ok(());
    }

    let has_docker = std::process::Command::new("docker")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_docker {
        eprintln!("Warning: docker not available, cannot install features");
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
        let mut seen = std::collections::HashSet::new();
        for id in order {
            if !seen.insert(id.clone()) {
                eprintln!(
                    "Warning: duplicate feature '{id}' in overrideFeatureInstallOrder, skipping duplicate"
                );
                continue;
            }
            if let Some(opts) = feat_map.get(id) {
                install_feature(id, opts, container_name, container_user)?;
            }
        }
        for (id, opts) in feat_map {
            if !order.contains(id) {
                install_feature(id, opts, container_name, container_user)?;
            }
        }
    } else {
        let sorted = sort_by_installs_after(feat_map);
        println!("Installing features in installsAfter order:");
        for id in sorted {
            if let Some(opts) = feat_map.get(&id) {
                install_feature(&id, opts, container_name, container_user)?;
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
        if let Some(feat_opts) = feat_map.get(id)
            && let Some(after) = feat_opts.get("installsAfter")
            && after.as_array().is_none()
        {
            eprintln!(
                "Warning: feature '{id}' installsAfter must be an array of strings, got {after}"
            );
        }
        if let Some(opts) = feat_map.get(id)
            && let Some(arr) = opts.get("installsAfter").and_then(|v| v.as_array())
        {
            for dep in arr {
                if let Some(dep_str) = dep.as_str() {
                    if !feat_map.contains_key(dep_str) {
                        eprintln!(
                            "Warning: feature '{id}' installsAfter references unknown feature '{dep_str}'"
                        );
                    } else {
                        visit(dep_str, feat_map, sorted, visiting, visited);
                    }
                }
            }
        }
        visiting.remove(id);
        visited.insert(id.to_string());
        sorted.push(id.to_string());
    }

    let mut ids: Vec<&String> = feat_map.keys().collect();
    ids.sort();
    for id in ids {
        visit(id, feat_map, &mut sorted, &mut visiting, &mut visited);
    }
    sorted
}

fn feature_cache_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("bondar_features")
}

fn sanitize_id(id: &str) -> String {
    // Distinguish separators so distinct IDs (e.g. "a/b" vs "a-b") do not
    // collide into the same directory name.
    id.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c
            } else if c == '/' {
                '-'
            } else {
                '_'
            }
        })
        .collect()
}

fn run_output(cmd: &mut std::process::Command, desc: &str) -> Result<(bool, String)> {
    let output = cmd
        .output()
        .map_err(|e| BondarError::Docker(format!("Failed to run {desc}: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.trim().is_empty() {
        print!("{stdout}");
    }
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
            ensure_extracted(dest_dir);
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

/// Some OCI registries return the feature as a tar archive. Expand it so
/// install.sh is directly accessible under dest_dir.
fn ensure_extracted(dest_dir: &Path) {
    if dest_dir.join("install.sh").exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dest_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(String::from) else {
            continue;
        };
        if name.ends_with(".tar.gz") || name.ends_with(".tgz") || name.ends_with(".tar") {
            println!("  Expanding archive {name}...");
            let status = std::process::Command::new("tar")
                .arg("-xf")
                .arg(&path)
                .arg("-C")
                .arg(dest_dir)
                .status();
            match status {
                Ok(s) if s.success() => {
                    println!("  Expanded {name}");
                    let _ = std::fs::remove_file(&path);
                }
                Ok(s) => {
                    eprintln!(
                        "  Warning: failed to expand {name} (tar exit {})",
                        s.code().unwrap_or(-1)
                    );
                }
                Err(e) => {
                    eprintln!("  Warning: failed to run tar for {name}: {e}");
                }
            }
        }
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
        let home = resolve_user_home(container, user);
        exec_cmd.arg("-e").arg(format!("_CONTAINER_USER={user}"));
        exec_cmd.arg("-e").arg(format!("_REMOTE_USER={user}"));
        exec_cmd.arg("-e").arg(format!("_USERNAME={user}"));
        exec_cmd
            .arg("-e")
            .arg(format!("_CONTAINER_USER_HOME={home}"));
        exec_cmd.arg("-e").arg(format!("_REMOTE_USER_HOME={home}"));
    }
    exec_cmd.arg(container);
    // Normalize CRLF line endings so install.sh does not fail with
    // "not found" when the file has Windows line endings.
    exec_cmd.arg("sh").arg("-c").arg(format!(
        "cd {container_path} && (sed -i 's/\\r$//' install.sh 2>/dev/null || tr -d '\\r' < install.sh > install.sh.tmp && mv install.sh.tmp install.sh 2>/dev/null || true) && chmod +x install.sh && ./install.sh"
    ));
    // Pass feature options as environment variables
    if let serde_json::Value::Object(map) = opts {
        for (k, v) in map {
            // `installsAfter` is metadata consumed by bondar, not an
            // environment variable for the install script
            if k == "installsAfter" {
                continue;
            }
            if v.is_null() {
                continue;
            }
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
    // also accept the plural form for compatibility.
    for name in ["devcontainer-feature.json", "devcontainer-features.json"] {
        let path = dir.join(name);
        if let Ok(content) = std::fs::read_to_string(path)
            && let Ok(value) = serde_json::from_str(&content)
        {
            return Some(value);
        }
    }
    None
}

/// Resolve the user's home directory inside the container via `getent passwd`,
/// falling back to `/home/{user}` when unavailable (e.g. no getent).
fn resolve_user_home(container: &str, user: &str) -> String {
    std::process::Command::new("docker")
        .args([
            "exec",
            container,
            "sh",
            "-c",
            &format!("getent passwd {user} | cut -d: -f6"),
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("/home/{user}"))
}

fn install_feature(
    id: &str,
    opts: &serde_json::Value,
    container_name: Option<&str>,
    container_user: Option<&str>,
) -> Result<()> {
    if !id.contains('/') && !id.contains('.') {
        eprintln!("Warning: feature ID '{id}' looks invalid, skipping");
        return Ok(());
    }

    println!("Attempting to install feature '{id}' with opts {opts}...");

    let dest_dir = feature_cache_dir().join(sanitize_id(id));
    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        eprintln!("  Warning: could not create feature directory: {e}");
        return Ok(());
    }
    if let Err(e) = fetch_feature(id, &dest_dir) {
        eprintln!("  Warning: could not fetch feature {id}: {e}");
        // Avoid stale metadata from a previous failed/partial fetch
        let _ = std::fs::remove_dir_all(&dest_dir);
        return Ok(());
    }

    // Read devcontainer-features.json metadata for installsAfter dependencies
    if let Some(meta) = read_feature_metadata(&dest_dir) {
        if let Some(after) = meta.get("installsAfter").and_then(|v| v.as_array()) {
            let deps: Vec<String> = after
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if !deps.is_empty() {
                println!("  Feature declares installsAfter: {deps:?}");
            }
        }
        // Report feature-declared requirements that need a container rebuild
        for key in [
            "containerEnv",
            "mounts",
            "privileged",
            "init",
            "capAdd",
            "securityOpt",
        ] {
            if let Some(val) = meta.get(key) {
                println!(
                    "  Note: feature declares {key} = {val}; a container rebuild ('bondar up --remove-existing-container') is required to apply it"
                );
            }
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
        assert!(handle_features_with_container(&None, &None, None, None).is_ok());
        let empty: HashMap<String, serde_json::Value> = HashMap::new();
        assert!(handle_features_with_container(&Some(empty), &None, None, None).is_ok());
    }

    #[test]
    fn test_sanitize_id() {
        assert_eq!(
            sanitize_id("ghcr.io/devcontainers/features/common-utils:2"),
            "ghcr_io-devcontainers-features-common_utils_2"
        );
    }

    #[test]
    fn test_sanitize_id_special_and_unicode() {
        assert_eq!(sanitize_id("a b@c"), "a_b_c");
        // Unicode alphanumerics are preserved
        assert_eq!(sanitize_id("日本語"), "日本語");
        assert_ne!(sanitize_id("ghcr.io/a/b"), sanitize_id("ghcr.io/a_b"));
    }

    #[test]
    fn test_feature_cache_dir() {
        let dir = feature_cache_dir();
        assert!(dir.starts_with(&std::env::temp_dir()));
        assert_eq!(dir.file_name().unwrap(), "bondar_features");
    }

    #[test]
    fn test_sanitize_id_no_collision() {
        assert_ne!(sanitize_id("ghcr.io/a/b"), sanitize_id("ghcr.io/a-b"));
    }

    #[test]
    fn test_read_feature_metadata_singular_and_plural() {
        let dir = std::env::temp_dir().join("bondar-feature-meta-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Singular form (spec) takes precedence
        std::fs::write(
            dir.join("devcontainer-feature.json"),
            r#"{"name": "x", "installsAfter": ["ghcr.io/a/y"]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("devcontainer-features.json"),
            r#"{"name": "x", "installsAfter": ["ghcr.io/a/z"]}"#,
        )
        .unwrap();
        let meta = read_feature_metadata(&dir).unwrap();
        assert_eq!(meta["installsAfter"][0], "ghcr.io/a/y");

        // Plural-only fallback
        let dir2 = std::env::temp_dir().join("bondar-feature-meta-test2");
        let _ = std::fs::remove_dir_all(&dir2);
        std::fs::create_dir_all(&dir2).unwrap();
        std::fs::write(dir2.join("devcontainer-features.json"), r#"{"name": "y"}"#).unwrap();
        assert!(read_feature_metadata(&dir2).is_some());

        // Missing -> None
        let dir3 = std::env::temp_dir().join("bondar-feature-meta-test3");
        let _ = std::fs::remove_dir_all(&dir3);
        std::fs::create_dir_all(&dir3).unwrap();
        assert!(read_feature_metadata(&dir3).is_none());

        // Invalid JSON -> None
        std::fs::write(dir3.join("devcontainer-feature.json"), "not json").unwrap();
        assert!(read_feature_metadata(&dir3).is_none());

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
        let _ = std::fs::remove_dir_all(&dir3);
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

    #[test]
    fn test_sort_by_installs_after_unknown_dependency() {
        // Unknown dependency is skipped (a warning is emitted), no hang
        let mut feat_map: HashMap<String, serde_json::Value> = HashMap::new();
        feat_map.insert(
            "ghcr.io/a/only".to_string(),
            serde_json::json!({ "installsAfter": ["ghcr.io/a/missing"] }),
        );
        let sorted = sort_by_installs_after(&feat_map);
        assert_eq!(sorted, vec!["ghcr.io/a/only"]);
    }

    #[test]
    fn test_sort_by_installs_after_non_array() {
        // Non-array installsAfter is warned and ignored
        let mut feat_map: HashMap<String, serde_json::Value> = HashMap::new();
        feat_map.insert(
            "ghcr.io/a/x".to_string(),
            serde_json::json!({ "installsAfter": "not-an-array" }),
        );
        let sorted = sort_by_installs_after(&feat_map);
        assert_eq!(sorted.len(), 1);
    }

    #[test]
    fn test_sort_by_installs_after_deterministic() {
        let mut feat_map: HashMap<String, serde_json::Value> = HashMap::new();
        feat_map.insert("ghcr.io/a/b".to_string(), serde_json::json!({}));
        feat_map.insert("ghcr.io/a/a".to_string(), serde_json::json!({}));
        let sorted1 = sort_by_installs_after(&feat_map);
        let sorted2 = sort_by_installs_after(&feat_map);
        assert_eq!(sorted1, sorted2);
        assert_eq!(sorted1, vec!["ghcr.io/a/a", "ghcr.io/a/b"]);
    }
}
