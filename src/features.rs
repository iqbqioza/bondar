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
        println!("Installing features in default order (no installsAfter handling):");
        for (id, opts) in feat_map {
            install_feature(id, opts)?;
        }
    }

    Ok(())
}

fn install_feature(id: &str, _opts: &serde_json::Value) -> Result<()> {
    // Bondar does not yet implement full feature installation.
    // Features are OCI artifacts that contain install.sh.
    // For now, we warn and skip, but we could attempt to use `devcontainer` feature handling via docker.
    // To keep standalone without Node.js, we at least validate the feature ID format.
    if !id.contains('/') && !id.contains('.') {
        eprintln!("Warning: feature ID '{id}' looks invalid, skipping");
        return Ok(());
    }
    eprintln!(
        "Warning: feature '{id}' installation is not yet fully implemented - skipping (would require OCI fetch and install.sh execution)"
    );
    // Future: fetch ghcr.io/devcontainers/features/... and run install.sh inside container
    // For now, we treat as warning to keep bondar standalone.
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
