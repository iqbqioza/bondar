use std::collections::HashMap;

use crate::error::Result;

pub fn handle_features(
    features: &Option<HashMap<String, serde_json::Value>>,
    override_order: &Option<Vec<String>>,
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
                install_feature(id, opts)?;
            }
        }
        for (id, opts) in feat_map {
            if !order.contains(id) {
                install_feature(id, opts)?;
            }
        }
    } else {
        let sorted = sort_by_installs_after(feat_map);
        println!("Installing features in installsAfter order:");
        for id in sorted {
            if let Some(opts) = feat_map.get(&id) {
                install_feature(&id, opts)?;
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

fn install_feature(id: &str, opts: &serde_json::Value) -> Result<()> {
    if !id.contains('/') && !id.contains('.') {
        eprintln!("Warning: feature ID '{id}' looks invalid, skipping");
        return Ok(());
    }

    // Try to handle via OCI if `oras` is available, otherwise fallback to docker pull attempt
    let has_oras = std::process::Command::new("oras")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

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

    if has_oras {
        println!("  Using 'oras' to fetch OCI artifact (if available)");
        let output = std::process::Command::new("oras")
            .args(["pull", id, "--output", "/tmp/bondar_features"])
            .output();
        if let Ok(o) = output {
            if o.status.success() {
                println!("  Fetched feature {id} via oras");
            } else {
                eprintln!(
                    "  Warning: oras pull failed for {id}: {}",
                    String::from_utf8_lossy(&o.stderr)
                );
            }
        }
    } else {
        // Fallback: try docker pull for GHCR features that are also Docker images (some are)
        let feature_image = id.split(':').next().unwrap_or(id);
        println!("  Trying 'docker pull {feature_image}' as fallback");
        let output = std::process::Command::new("docker")
            .args(["pull", feature_image])
            .output();
        if let Ok(o) = output {
            if o.status.success() {
                println!("  Pulled feature image {feature_image}");
            } else {
                eprintln!(
                    "  Note: docker pull failed for {feature_image} (expected for pure OCI features): {}",
                    String::from_utf8_lossy(&o.stderr)
                        .lines()
                        .next()
                        .unwrap_or("")
                );
            }
        }
    }

    eprintln!(
        "  Note: Full feature installation requires executing install.sh inside the dev container with _CONTAINER_USER and options. Bondar currently validates and fetches but skips execution to remain standalone. Feature '{id}' treated as validated."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_handle_empty() {
        assert!(handle_features(&None, &None).is_ok());
        let empty: HashMap<String, serde_json::Value> = HashMap::new();
        assert!(handle_features(&Some(empty), &None).is_ok());
    }
}
