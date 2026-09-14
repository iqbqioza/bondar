use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{BondarError, Result};

/// Collect `customizations` from all fetched feature metadata files and merge
/// them into a single object. Returns an empty map when nothing is available.
pub fn collect_feature_customizations(
    feat_map: &HashMap<String, serde_json::Value>,
    order: &[String],
) -> serde_json::Value {
    let mut merged = serde_json::Map::new();
    for id in feature_ids_in_order(feat_map, order) {
        let dir = feature_cache_dir().join(sanitize_id(&id));
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
                merge_customization_values(existing, incoming);
            }
        }
    }
    serde_json::Value::Object(merged)
}

/// Merge one feature's customization namespace into the accumulated one:
/// objects are merged recursively, arrays are set as a union and other values
/// are replaced (per spec).
fn merge_customization_values(
    existing: &mut serde_json::Map<String, serde_json::Value>,
    incoming: &serde_json::Map<String, serde_json::Value>,
) {
    for (k, v) in incoming {
        match (existing.get_mut(k), v) {
            (Some(serde_json::Value::Object(dest)), serde_json::Value::Object(src)) => {
                merge_customization_values(dest, src);
            }
            (Some(serde_json::Value::Array(dest)), serde_json::Value::Array(src)) => {
                for item in src {
                    if !dest.contains(item) {
                        dest.push(item.clone());
                    }
                }
            }
            _ => {
                existing.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Container properties a feature may declare in its metadata. They must be
/// merged into the container configuration before the container is created.
#[derive(Debug, Default, Clone)]
pub struct FeatureContainerProperties {
    pub container_env: HashMap<String, String>,
    pub mounts: Vec<crate::config::MountValue>,
    pub privileged: bool,
    pub init: bool,
    pub cap_add: Vec<String>,
    pub security_opt: Vec<String>,
}

/// Parse the container properties from a fetched feature's metadata.
fn collect_feature_container_properties(id: &str, dir: &Path) -> FeatureContainerProperties {
    let mut props = FeatureContainerProperties::default();
    let Some(meta) = read_feature_metadata(dir) else {
        return props;
    };
    if let Some(env) = meta.get("containerEnv").and_then(|v| v.as_object()) {
        for (k, v) in env {
            let value = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => v.to_string(),
            };
            props.container_env.insert(k.clone(), value);
        }
    }
    if let Some(mounts) = meta.get("mounts").and_then(|v| v.as_array()) {
        for mount in mounts {
            match serde_json::from_value::<crate::config::MountValue>(mount.clone()) {
                Ok(m) => props.mounts.push(m),
                Err(e) => {
                    eprintln!(
                        "  Warning: feature '{id}' mount {mount} is invalid and was ignored: {e}"
                    );
                }
            }
        }
    }
    if let Some(v) = meta.get("privileged").and_then(|v| v.as_bool()) {
        props.privileged = v;
    }
    if let Some(v) = meta.get("init").and_then(|v| v.as_bool()) {
        props.init = v;
    }
    for (key, target) in [
        ("capAdd", &mut props.cap_add),
        ("securityOpt", &mut props.security_opt),
    ] {
        if let Some(arr) = meta.get(key).and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    target.push(s.to_string());
                }
            }
        }
    }
    props
}

/// Merge one feature's properties into the accumulator. Dependencies are
/// processed first, so a dependent feature overrides containerEnv values and
/// appends to mounts/capabilities.
fn merge_feature_container_properties(
    acc: &mut FeatureContainerProperties,
    next: FeatureContainerProperties,
) {
    for (k, v) in next.container_env {
        acc.container_env.insert(k, v);
    }
    acc.mounts.extend(next.mounts);
    acc.privileged = acc.privileged || next.privileged;
    acc.init = acc.init || next.init;
    for c in next.cap_add {
        if !acc.cap_add.contains(&c) {
            acc.cap_add.push(c);
        }
    }
    for s in next.security_opt {
        if !acc.security_opt.contains(&s) {
            acc.security_opt.push(s);
        }
    }
}

/// Fetch all configured features (and their `dependsOn` dependencies) into the
/// cache and collect the container properties they declare, so the container
/// can be created with them.
pub fn prefetch_feature_container_properties(
    features: &Option<HashMap<String, serde_json::Value>>,
) -> Result<(FeatureContainerProperties, Vec<String>)> {
    let Some(feat_map) = features else {
        return Ok((FeatureContainerProperties::default(), Vec::new()));
    };
    if feat_map.is_empty() {
        return Ok((FeatureContainerProperties::default(), Vec::new()));
    }
    let has_docker = std::process::Command::new("docker")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_docker {
        eprintln!("Warning: docker not available, cannot read feature metadata");
        return Ok((FeatureContainerProperties::default(), Vec::new()));
    }
    let mut props = FeatureContainerProperties::default();
    let mut prefetched = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut visiting = std::collections::HashSet::new();
    let mut ids: Vec<&String> = feat_map.keys().collect();
    ids.sort();
    for id in ids {
        prefetch_one_feature(id, &mut props, &mut prefetched, &mut visited, &mut visiting)?;
    }
    Ok((props, prefetched))
}

fn prefetch_one_feature(
    id: &str,
    props: &mut FeatureContainerProperties,
    prefetched: &mut Vec<String>,
    visited: &mut std::collections::HashSet<String>,
    visiting: &mut std::collections::HashSet<String>,
) -> Result<()> {
    if visited.contains(id) {
        return Ok(());
    }
    if !visiting.insert(id.to_string()) {
        eprintln!("Warning: circular dependsOn detected for feature '{id}'; skipping dependency");
        return Ok(());
    }
    let result = prefetch_one_feature_inner(id, props, prefetched, visited, visiting);
    visiting.remove(id);
    result
}

fn prefetch_one_feature_inner(
    id: &str,
    props: &mut FeatureContainerProperties,
    prefetched: &mut Vec<String>,
    visited: &mut std::collections::HashSet<String>,
    visiting: &mut std::collections::HashSet<String>,
) -> Result<()> {
    if !id.contains('/') && !id.contains('.') {
        eprintln!("Warning: feature ID '{id}' looks invalid, skipping");
        return Ok(());
    }
    let Some(dest_dir) = fetch_feature_to_cache(id)? else {
        return Ok(());
    };
    prefetched.push(id.to_string());
    if let Some(meta) = read_feature_metadata(&dest_dir) {
        if meta
            .get("deprecated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            eprintln!("Warning: feature '{id}' is marked as deprecated by its author");
        }
        if let Some(deps) = meta.get("dependsOn").and_then(|v| v.as_object()) {
            let mut dep_ids: Vec<&String> = deps.keys().collect();
            dep_ids.sort();
            for dep_id in dep_ids {
                if dep_id.as_str() == id {
                    eprintln!("Warning: feature '{id}' dependsOn itself; skipping");
                    continue;
                }
                prefetch_one_feature(dep_id, props, prefetched, visited, visiting)?;
            }
        }
    }
    let resolved_props = collect_feature_container_properties(id, &dest_dir);
    merge_feature_container_properties(props, resolved_props);
    visited.insert(id.to_string());
    Ok(())
}

/// Merge feature-declared container properties into the configuration. User
/// configuration wins for `containerEnv`; feature requirements can only enable
/// `privileged`/`init` and append capabilities/security options.
pub fn apply_feature_container_properties(
    config: &mut crate::config::DevContainerConfig,
    props: &FeatureContainerProperties,
) {
    for (k, v) in &props.container_env {
        config
            .container_env
            .entry(k.clone())
            .or_insert_with(|| v.clone());
    }
    for m in &props.mounts {
        config.mounts.push(m.clone());
    }
    if props.privileged {
        config.privileged = Some(true);
    }
    if props.init {
        config.init = Some(true);
    }
    for c in &props.cap_add {
        if !config.cap_add.contains(c) {
            config.cap_add.push(c.clone());
        }
    }
    for s in &props.security_opt {
        if !config.security_opt.contains(s) {
            config.security_opt.push(s.clone());
        }
    }
}
/// Lifecycle hooks that features may declare in their metadata, in spec order.
pub const FEATURE_LIFECYCLE_HOOKS: [&str; 5] = [
    "onCreateCommand",
    "updateContentCommand",
    "postCreateCommand",
    "postStartCommand",
    "postAttachCommand",
];

/// Feature ids in installation order: the given order first (which includes
/// dependencies), then any requested features not present in it, sorted for
/// determinism.
fn feature_ids_in_order(
    feat_map: &HashMap<String, serde_json::Value>,
    order: &[String],
) -> Vec<String> {
    let mut ids: Vec<String> = order.to_vec();
    let mut extra: Vec<String> = feat_map
        .keys()
        .filter(|id| !order.contains(id))
        .cloned()
        .collect();
    extra.sort();
    ids.extend(extra);
    ids
}

/// Sorted feature ids for cases where no installation happened (e.g. a
/// container restart), so cached metadata is still processed deterministically.
pub fn feature_ids_sorted(features: &Option<HashMap<String, serde_json::Value>>) -> Vec<String> {
    // Traverse `dependsOn` from cached metadata so dependencies are included
    // (and run first) even when no installation happens in this invocation
    // (e.g. a container restart).
    fn visit(id: &str, result: &mut Vec<String>, visiting: &mut std::collections::HashSet<String>) {
        if result.iter().any(|existing| existing == id) {
            return;
        }
        if !visiting.insert(id.to_string()) {
            return;
        }
        let dir = feature_cache_dir().join(sanitize_id(id));
        if let Some(meta) = read_feature_metadata(&dir)
            && let Some(deps) = meta.get("dependsOn").and_then(|v| v.as_object())
        {
            let mut dep_ids: Vec<&String> = deps.keys().collect();
            dep_ids.sort();
            for dep_id in dep_ids {
                if dep_id.as_str() != id {
                    visit(dep_id, result, visiting);
                }
            }
        }
        visiting.remove(id);
        if !result.iter().any(|existing| existing == id) {
            result.push(id.to_string());
        }
    }

    let mut seeds: Vec<String> = features
        .as_ref()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    seeds.sort();
    let mut result = Vec::new();
    let mut visiting = std::collections::HashSet::new();
    for id in &seeds {
        visit(id, &mut result, &mut visiting);
    }
    result
}

/// Collect lifecycle commands declared by features (from cached metadata), in
/// deterministic feature order, so they can run before the user's lifecycle
/// commands.
pub fn collect_feature_lifecycle_hooks(
    feat_map: &HashMap<String, serde_json::Value>,
    order: &[String],
) -> Vec<(&'static str, serde_json::Value)> {
    let mut hooks = Vec::new();
    for id in feature_ids_in_order(feat_map, order) {
        let dir = feature_cache_dir().join(sanitize_id(&id));
        let Some(meta) = read_feature_metadata(&dir) else {
            continue;
        };
        for hook in FEATURE_LIFECYCLE_HOOKS {
            if let Some(value) = meta.get(hook)
                && !value.is_null()
            {
                hooks.push((hook, value.clone()));
            }
        }
    }
    hooks
}

pub fn handle_features_with_container(
    features: &Option<HashMap<String, serde_json::Value>>,
    override_order: &Option<Vec<String>>,
    prefetched: &[String],
    container_name: Option<&str>,
    remote_user: Option<&str>,
    container_user: Option<&str>,
) -> Result<(HashMap<String, serde_json::Value>, Vec<String>)> {
    let Some(feat_map) = features else {
        return Ok((HashMap::new(), Vec::new()));
    };
    if feat_map.is_empty() {
        return Ok((HashMap::new(), Vec::new()));
    }

    let has_docker = std::process::Command::new("docker")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_docker {
        eprintln!("Warning: docker not available, cannot install features");
        return Ok((HashMap::new(), Vec::new()));
    }

    println!("Features requested: {} feature(s)", feat_map.len());
    for (id, opts) in feat_map {
        println!("  - {id}: {opts}");
    }

    let mut installer =
        FeatureInstaller::new(container_name, remote_user, container_user, prefetched);

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
                installer.install(id, opts)?;
            }
        }
        let mut remaining: Vec<&String> =
            feat_map.keys().filter(|id| !order.contains(*id)).collect();
        remaining.sort();
        for id in remaining {
            if let Some(opts) = feat_map.get(id) {
                installer.install(id, opts)?;
            }
        }
    } else {
        // Merge installsAfter from cached feature metadata (if any) into the
        // ordering map, so a feature's own `installsAfter` is honored even
        // before it is fetched.
        let mut ordered_map = (*feat_map).clone();
        for id in feat_map.keys() {
            let dir = feature_cache_dir().join(sanitize_id(id));
            if let Some(meta) = read_feature_metadata(&dir)
                && let Some(arr) = meta.get("installsAfter").and_then(|v| v.as_array())
            {
                let entry = ordered_map
                    .entry(id.clone())
                    .or_insert(serde_json::json!({}));
                let needs_insert = entry.get("installsAfter").is_none();
                if needs_insert {
                    if let Some(obj) = entry.as_object_mut() {
                        obj.insert(
                            "installsAfter".to_string(),
                            serde_json::Value::Array(arr.clone()),
                        );
                    } else {
                        *entry = serde_json::json!({ "installsAfter": arr.clone() });
                    }
                }
            }
        }
        let sorted = sort_by_installs_after(&ordered_map);
        println!("Installing features in installsAfter order:");
        for id in sorted {
            if let Some(opts) = feat_map.get(&id) {
                installer.install(&id, opts)?;
            }
        }
    }

    Ok((installer.installed, installer.order))
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
    // The readable part distinguishes common separators; the FNV-1a suffix
    // keeps the mapping collision-free for IDs that sanitize alike
    // (e.g. "ghcr.io/a-b" and "ghcr.io/a_b").
    let readable: String = id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c
            } else if c == '/' {
                '-'
            } else {
                '_'
            }
        })
        .collect();
    let mut hash: u64 = 14695981039346656037;
    for b in id.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("{readable}-{:08x}", hash as u32)
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
        let dest_str = dest_dir.to_str().ok_or_else(|| {
            BondarError::Config("Feature cache path is not valid UTF-8".to_string())
        })?;
        let (ok, stderr) = run_output(
            std::process::Command::new("oras").args(["pull", id, "--output", dest_str]),
            &format!("oras pull {id}"),
        )?;
        if ok {
            println!("  Fetched feature {id} via oras");
            ensure_extracted(dest_dir);
            return Ok(());
        }
        eprintln!("  Warning: oras pull failed: {stderr}");
        // Fall through to the docker pull fallback
    }

    // Fallback: docker pull (works for features that are also container images).
    // Feature IDs are valid image references (e.g. ghcr.io/devcontainers/features/common-utils:2),
    // so the full ID is used to keep the requested tag/version.
    if id.starts_with('-') {
        return Err(BondarError::Config(format!(
            "Feature id '{id}' must not start with '-'"
        )));
    }
    let feature_image = id;
    println!("  Trying 'docker pull {feature_image}' as fallback");
    let (ok, stderr) = run_output(
        std::process::Command::new("docker").args(["pull", "--", feature_image]),
        "docker pull",
    )?;
    if ok {
        println!("  Pulled feature image {feature_image}; extracting feature files...");
        let id_suffix: String = sanitize_id(id).chars().take(64).collect();
        let tmp_name = format!("bondar-feature-extract-{}-{id_suffix}", std::process::id());
        let dest_str = dest_dir.to_str().ok_or_else(|| {
            BondarError::Config("Feature cache path is not valid UTF-8".to_string())
        })?;
        // Feature images are often file-only OCI artifacts with no CMD; pass a
        // harmless command so `docker create` accepts them (the container is
        // never started, only used for `docker cp`).
        let created = std::process::Command::new("docker")
            .args(["create", "--name", &tmp_name, "--", feature_image, "sh"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        // Feature images typically carry /install.sh at the root; copy only
        // that file instead of the whole root filesystem (which may include
        // mount points that docker cp cannot handle).
        let cp_install = created
            && std::process::Command::new("docker")
                .args(["cp", &format!("{tmp_name}:/install.sh"), dest_str])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
        let extracted = if cp_install {
            // Best effort: without the metadata file, option defaults,
            // customizations and installsAfter declared by the feature are lost
            for file in ["devcontainer-feature.json", "devcontainer-features.json"] {
                let _ = std::process::Command::new("docker")
                    .args(["cp", &format!("{tmp_name}:/{file}"), dest_str])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
            true
        } else {
            created
                && std::process::Command::new("docker")
                    .args(["cp", &format!("{tmp_name}:/"), dest_str])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
        };
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", &tmp_name])
            .status();
        if extracted {
            println!("  Extracted feature files from image");
            ensure_extracted(dest_dir);
            return Ok(());
        }
        eprintln!("  Warning: could not extract feature files from pulled image");
        Err(BondarError::Docker(format!(
            "Unable to fetch feature {id} (no oras, docker pull extraction failed)"
        )))
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
        let lower_name = name.to_lowercase();
        if lower_name.ends_with(".tar.gz")
            || lower_name.ends_with(".tgz")
            || lower_name.ends_with(".tar")
        {
            // Guard against path traversal / absolute paths in the archive
            if let Ok(list) = std::process::Command::new("tar")
                .arg("-tf")
                .arg(&path)
                .output()
            {
                let listing = String::from_utf8_lossy(&list.stdout);
                if listing
                    .lines()
                    .any(|l| l.starts_with('/') || l.split('/').any(|c| c == ".."))
                {
                    eprintln!(
                        "  Warning: archive {name} contains unsafe paths; skipping expansion"
                    );
                    continue;
                }
            }
            // Reject archives containing symlinks (they can escape dest_dir)
            if let Ok(list) = std::process::Command::new("tar")
                .arg("-tvf")
                .arg(&path)
                .output()
            {
                let listing = String::from_utf8_lossy(&list.stdout);
                if listing.lines().any(|l| {
                    l.split_whitespace()
                        .next()
                        .map(|mode| mode.starts_with('l'))
                        .unwrap_or(false)
                }) {
                    eprintln!("  Warning: archive {name} contains symlinks; skipping expansion");
                    continue;
                }
            }
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
            "--user",
            "root",
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

    let host_str = host_dir
        .to_str()
        .ok_or_else(|| BondarError::Config("Feature cache path is not valid UTF-8".to_string()))?;
    // The `/.` suffix copies the directory *contents* into container_path
    // (without it, docker cp would create container_path/<basename>).
    let (ok, stderr) = run_output(
        std::process::Command::new("docker").args([
            "cp",
            &format!("{host_str}/."),
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
    remote_user: Option<&str>,
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
    // install.sh always runs as root; the target users are passed via env
    exec_cmd.arg("--user").arg("root");
    // Per spec: _CONTAINER_USER is the container's user, _REMOTE_USER is the
    // configured remoteUser; when only one is set it is used for both, and
    // when neither is configured the container's own user is used.
    let fallback_user = if container_user.is_none() && remote_user.is_none() {
        Some(resolve_container_user(container))
    } else {
        None
    };
    let effective_container_user = container_user.or(remote_user).or(fallback_user.as_deref());
    let effective_remote_user = remote_user.or(container_user).or(fallback_user.as_deref());
    if let Some(user) = effective_container_user {
        let home = resolve_user_home(container, user);
        exec_cmd.arg("-e").arg(format!("_CONTAINER_USER={user}"));
        exec_cmd
            .arg("-e")
            .arg(format!("_CONTAINER_USER_HOME={home}"));
    }
    if let Some(user) = effective_remote_user {
        let home = resolve_user_home(container, user);
        exec_cmd.arg("-e").arg(format!("_REMOTE_USER={user}"));
        exec_cmd.arg("-e").arg(format!("_REMOTE_USER_HOME={home}"));
        exec_cmd.arg("-e").arg(format!("_USERNAME={user}"));
    }
    // Pass feature options as environment variables. NOTE: all `-e` flags must
    // come before the container name; `docker exec [OPTIONS] CONTAINER ...`
    // treats everything after the container name as the command.
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
            exec_cmd
                .arg("-e")
                .arg(format!("{}={value}", option_env_name(k)));
        }
    }
    exec_cmd.arg(container);
    // Normalize CRLF line endings so install.sh does not fail with
    // "not found" when the file has Windows line endings.
    exec_cmd.arg("sh").arg("-c").arg(format!(
        "cd {container_path} && (sed -i 's/\\r$//' install.sh 2>/dev/null || tr -d '\\r' < install.sh > install.sh.tmp && mv install.sh.tmp install.sh 2>/dev/null || true) && chmod +x install.sh && ./install.sh"
    ));

    let (ok, stderr) = run_output(&mut exec_cmd, "install.sh")?;
    if ok {
        println!("  Feature {id} installed successfully");
    } else {
        let _ = run_output(
            std::process::Command::new("docker").args([
                "exec",
                "--user",
                "root",
                container,
                "sh",
                "-c",
                &format!("rm -rf {container_path}"),
            ]),
            "cleanup",
        );
        return Err(BondarError::Docker(format!(
            "install.sh failed for {id}: {stderr}"
        )));
    }

    // Cleanup the copied files inside the container
    let _ = run_output(
        std::process::Command::new("docker").args([
            "exec",
            "--user",
            "root",
            container,
            "sh",
            "-c",
            &format!("rm -rf {container_path}"),
        ]),
        "cleanup",
    );

    Ok(())
}

/// Convert a feature option name to its environment variable form, per the
/// devcontainer spec: non-word characters become `_`, a leading run of digits
/// and underscores collapses to a single `_`, then the result is uppercased.
fn option_env_name(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = mapped.trim_start_matches(|c: char| c.is_ascii_digit() || c == '_');
    let prefixed = if trimmed.len() == mapped.len() {
        mapped
    } else {
        format!("_{trimmed}")
    };
    prefixed.to_ascii_uppercase()
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

/// String form of a feature option value (booleans/numbers are stringified so
/// they can be compared with `enum` entries declared as strings).
fn option_value_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// User-provided feature option values that are not allowed by the feature's
/// declared `enum` (the spec treats these as strict values).
fn invalid_feature_option_values(id: &str, opts: &serde_json::Value, dir: &Path) -> Vec<String> {
    let Some(meta) = read_feature_metadata(dir) else {
        return Vec::new();
    };
    let Some(declared) = meta.get("options").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let Some(provided) = opts.as_object() else {
        return Vec::new();
    };
    let mut messages = Vec::new();
    for (name, spec) in declared {
        let Some(value) = provided.get(name) else {
            continue;
        };
        let Some(allowed) = spec.get("enum").and_then(|v| v.as_array()) else {
            continue;
        };
        let value_str = option_value_string(value);
        if !allowed
            .iter()
            .any(|allowed| option_value_string(allowed) == value_str)
        {
            let allowed: Vec<String> = allowed.iter().map(option_value_string).collect();
            messages.push(format!(
                "Warning: feature '{id}' option '{name}' value '{value_str}' is not one of the allowed values {allowed:?}"
            ));
        }
    }
    messages
}

/// Warn when a user-provided feature option value is not allowed by the
/// feature's declared `enum`.
fn warn_invalid_feature_option_values(id: &str, opts: &serde_json::Value, dir: &Path) {
    for message in invalid_feature_option_values(id, opts, dir) {
        eprintln!("{message}");
    }
}

/// Fill in defaults declared in the feature metadata for options the user
/// omitted; the spec requires omitted options to be exported with their
/// default values when `install.sh` runs. User-provided values win and options
/// without a default are left unset.
fn merge_feature_option_defaults(opts: &serde_json::Value, dir: &Path) -> serde_json::Value {
    let Some(meta) = read_feature_metadata(dir) else {
        return opts.clone();
    };
    let Some(declared) = meta.get("options").and_then(|v| v.as_object()) else {
        return opts.clone();
    };
    let mut merged = match opts {
        serde_json::Value::Object(map) => map.clone(),
        _ => serde_json::Map::new(),
    };
    for (name, spec) in declared {
        if merged.contains_key(name) {
            continue;
        }
        if let Some(default) = spec.get("default")
            && !default.is_null()
        {
            merged.insert(name.clone(), default.clone());
        }
    }
    serde_json::Value::Object(merged)
}

/// The user configured for the container (image `USER`, Dockerfile or compose),
/// falling back to `root` like Docker does when no user is set.
fn resolve_container_user(container: &str) -> String {
    std::process::Command::new("docker")
        .args(["inspect", "--format", "{{.Config.User}}", container])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .split(':')
                .next()
                .unwrap_or("")
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "root".to_string())
}

/// Resolve the user's home directory inside the container via `getent passwd`,
/// falling back to `/home/{user}` when unavailable (e.g. no getent).
/// The user is passed as a separate argument (no shell interpolation).
fn resolve_user_home(container: &str, user: &str) -> String {
    std::process::Command::new("docker")
        .args(["exec", container, "getent", "passwd", user])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .split(':')
                .nth(5)
                .unwrap_or("")
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("/home/{user}"))
}

/// Convert a user-provided feature option value to the effective options.
/// Per spec, a string value is shorthand for the `version` option:
/// "features": {"id": "18"} == {"id": {"version": "18"}}
fn effective_feature_opts(id: &str, opts: &serde_json::Value) -> serde_json::Value {
    if let Some(v) = opts.as_str() {
        serde_json::json!({ "version": v })
    } else {
        if !opts.is_object() && !opts.is_null() {
            eprintln!(
                "Warning: feature '{id}' options must be an object, got {opts}; ignoring options"
            );
        }
        opts.clone()
    }
}

/// Fetch a feature into the cache. `None` means the feature was skipped
/// (e.g. its cache directory could not be created).
fn fetch_feature_to_cache(id: &str) -> Result<Option<PathBuf>> {
    let dest_dir = feature_cache_dir().join(sanitize_id(id));
    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        eprintln!("  Warning: could not create feature directory: {e}");
        return Ok(None);
    }
    if let Err(e) = fetch_feature(id, &dest_dir) {
        // Avoid stale metadata from a previous failed/partial fetch
        let _ = std::fs::remove_dir_all(&dest_dir);
        return Err(e);
    }
    Ok(Some(dest_dir))
}

/// Install a feature that is already present in the cache.
fn install_fetched_feature(
    id: &str,
    dest_dir: &Path,
    opts: &serde_json::Value,
    container_name: Option<&str>,
    remote_user: Option<&str>,
    container_user: Option<&str>,
) -> Result<()> {
    if let Some(meta) = read_feature_metadata(dest_dir)
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

    // Warn about user-provided option values that the feature does not allow
    warn_invalid_feature_option_values(id, opts, dest_dir);

    // Apply defaults for omitted options declared in the feature metadata
    let effective_opts = merge_feature_option_defaults(opts, dest_dir);

    if let Some(container) = container_name {
        let container_path = format!("/tmp/bondar_features/{}", sanitize_id(id));
        if let Err(e) = copy_feature_into_container(dest_dir, container, &container_path) {
            eprintln!("  Warning: could not copy feature into container: {e}");
            return Ok(());
        }
        install_in_container(
            id,
            &effective_opts,
            container,
            &container_path,
            remote_user,
            container_user,
        )?;
    } else {
        println!(
            "  Feature {id} fetched to {}. Execution requires a running container (use 'bondar up' first).",
            dest_dir.display()
        );
    }

    Ok(())
}

/// Installs features (and their `dependsOn` dependencies) into the container,
/// depth-first so hard dependencies are always installed first.
struct FeatureInstaller<'a> {
    container_name: Option<&'a str>,
    remote_user: Option<&'a str>,
    container_user: Option<&'a str>,
    prefetched: &'a [String],
    installed: HashMap<String, serde_json::Value>,
    order: Vec<String>,
    visiting: std::collections::HashSet<String>,
}

impl<'a> FeatureInstaller<'a> {
    fn new(
        container_name: Option<&'a str>,
        remote_user: Option<&'a str>,
        container_user: Option<&'a str>,
        prefetched: &'a [String],
    ) -> Self {
        Self {
            container_name,
            remote_user,
            container_user,
            prefetched,
            installed: HashMap::new(),
            order: Vec::new(),
            visiting: std::collections::HashSet::new(),
        }
    }

    fn install(&mut self, id: &str, opts: &serde_json::Value) -> Result<()> {
        if self.installed.contains_key(id) {
            return Ok(());
        }
        if !self.visiting.insert(id.to_string()) {
            eprintln!(
                "Warning: circular dependsOn detected for feature '{id}'; skipping dependency"
            );
            return Ok(());
        }
        let result = self.install_new(id, opts);
        self.visiting.remove(id);
        result
    }

    fn install_new(&mut self, id: &str, opts: &serde_json::Value) -> Result<()> {
        if !id.contains('/') && !id.contains('.') {
            eprintln!("Warning: feature ID '{id}' looks invalid, skipping");
            return Ok(());
        }
        let effective_opts = effective_feature_opts(id, opts);
        println!("Attempting to install feature '{id}' with opts {effective_opts}...");

        // Reuse an artifact fetched by the pre-fetch pass (same run) instead
        // of downloading the feature a second time.
        let dest_dir = if self.prefetched.iter().any(|p| p.as_str() == id) {
            let dir = feature_cache_dir().join(sanitize_id(id));
            if dir.is_dir() {
                Some(dir)
            } else {
                fetch_feature_to_cache(id)?
            }
        } else {
            fetch_feature_to_cache(id)?
        };
        let Some(dest_dir) = dest_dir else {
            return Ok(());
        };

        // Hard dependencies declared in the feature metadata install first
        if let Some(meta) = read_feature_metadata(&dest_dir)
            && let Some(deps) = meta.get("dependsOn").and_then(|v| v.as_object())
        {
            for (dep_id, dep_opts) in deps {
                if dep_id.as_str() == id {
                    eprintln!("Warning: feature '{id}' dependsOn itself; skipping");
                    continue;
                }
                self.install(dep_id, dep_opts)?;
            }
        }

        install_fetched_feature(
            id,
            &dest_dir,
            &effective_opts,
            self.container_name,
            self.remote_user,
            self.container_user,
        )?;
        self.order.push(id.to_string());
        self.installed.insert(id.to_string(), effective_opts);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_handle_empty() {
        assert!(handle_features_with_container(&None, &None, &[], None, None, None).is_ok());
        let empty: HashMap<String, serde_json::Value> = HashMap::new();
        assert!(handle_features_with_container(&Some(empty), &None, &[], None, None, None).is_ok());
    }

    #[test]
    fn test_option_env_name() {
        assert_eq!(option_env_name("installZsh"), "INSTALLZSH");
        assert_eq!(option_env_name("version"), "VERSION");
        assert_eq!(option_env_name("foo-bar"), "FOO_BAR");
        assert_eq!(option_env_name("foo.bar"), "FOO_BAR");
        assert_eq!(option_env_name("1abc"), "_ABC");
        assert_eq!(option_env_name("_x"), "_X");
        assert_eq!(option_env_name("123"), "_");
        assert_eq!(option_env_name(""), "");
    }

    #[test]
    fn test_effective_feature_opts() {
        // String option values are shorthand for the `version` option
        assert_eq!(
            effective_feature_opts("ghcr.io/a/b", &serde_json::json!("18")),
            serde_json::json!({"version": "18"})
        );
        assert_eq!(
            effective_feature_opts("ghcr.io/a/b", &serde_json::json!({"a": 1})),
            serde_json::json!({"a": 1})
        );
        assert_eq!(
            effective_feature_opts("ghcr.io/a/b", &serde_json::Value::Null),
            serde_json::Value::Null
        );
    }

    #[test]
    fn test_sanitize_id() {
        let id = "ghcr.io/devcontainers/features/common-utils:2";
        let sanitized = sanitize_id(id);
        assert!(sanitized.starts_with("ghcr_io-devcontainers-features-common_utils_2-"));
        // Stable for the same id
        assert_eq!(sanitized, sanitize_id(id));
    }

    #[test]
    fn test_sanitize_id_special_and_unicode() {
        assert!(sanitize_id("a b@c").starts_with("a_b_c-"));
        // Unicode alphanumerics are preserved in the readable part
        assert!(sanitize_id("日本語").starts_with("日本語-"));
        assert!(sanitize_id("").starts_with('-'));
    }

    #[test]
    fn test_feature_cache_dir() {
        let dir = feature_cache_dir();
        let temp_dir = std::env::temp_dir();
        assert!(dir.starts_with(&temp_dir));
        assert_eq!(dir.file_name().unwrap(), "bondar_features");
    }

    #[test]
    fn test_sanitize_id_no_collision() {
        assert_ne!(sanitize_id("ghcr.io/a/b"), sanitize_id("ghcr.io/a-b"));
        // Separators that previously sanitized to the same string
        assert_ne!(sanitize_id("ghcr.io/a-b"), sanitize_id("ghcr.io/a_b"));
        assert_ne!(sanitize_id("ghcr.io/a/b"), sanitize_id("ghcr.io/a_b"));
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

        // A directory named like the metadata file -> None (is_file)
        let dir4 = std::env::temp_dir().join("bondar-feature-meta-test4");
        let _ = std::fs::remove_dir_all(&dir4);
        std::fs::create_dir_all(dir4.join("devcontainer-feature.json")).unwrap();
        assert!(read_feature_metadata(&dir4).is_none());

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
        let _ = std::fs::remove_dir_all(&dir3);
        let _ = std::fs::remove_dir_all(&dir4);
    }

    #[test]
    fn test_merge_feature_option_defaults() {
        let dir = std::env::temp_dir().join("bondar-feature-defaults-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("devcontainer-feature.json"),
            r#"{"id":"x","version":"1.0.0","name":"x","options":{"installZsh":{"type":"boolean","default":true},"version":{"type":"string","default":"latest"},"noDefault":{"type":"string"}}}"#,
        )
        .unwrap();

        // User values win; missing options get their declared defaults
        let merged = merge_feature_option_defaults(&serde_json::json!({"version": "18"}), &dir);
        assert_eq!(merged["version"], "18");
        assert_eq!(merged["installZsh"], true);
        assert!(merged.get("noDefault").is_none());

        // No user options -> all defaults; non-object opts are replaced
        let merged2 = merge_feature_option_defaults(&serde_json::Value::Null, &dir);
        assert_eq!(merged2["installZsh"], true);
        assert_eq!(merged2["version"], "latest");

        // Missing metadata -> unchanged
        let empty = std::env::temp_dir().join("bondar-feature-defaults-empty");
        let _ = std::fs::remove_dir_all(&empty);
        std::fs::create_dir_all(&empty).unwrap();
        let unchanged = merge_feature_option_defaults(&serde_json::json!({"a": 1}), &empty);
        assert_eq!(unchanged, serde_json::json!({"a": 1}));

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn test_collect_feature_customizations_unions_arrays() {
        let id_a = "ghcr.io/test/feature-union-a";
        let id_b = "ghcr.io/test/feature-union-b";
        for (id, key) in [(id_a, "a"), (id_b, "b")] {
            let dir = feature_cache_dir().join(sanitize_id(id));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("devcontainer-feature.json"),
                format!(
                    r#"{{"customizations":{{"vscode":{{"extensions":["{key}"],"settings":{{"{key}":1}}}}}}}}"#
                ),
            )
            .unwrap();
        }
        let features = HashMap::from([
            (id_a.to_string(), serde_json::json!({})),
            (id_b.to_string(), serde_json::json!({})),
        ]);
        let order = feature_ids_sorted(&Some(features.clone()));
        let merged = collect_feature_customizations(&features, &order);
        let extensions = merged["vscode"]["extensions"].as_array().unwrap();
        assert_eq!(extensions.len(), 2);
        assert!(extensions.contains(&serde_json::json!("a")));
        assert!(extensions.contains(&serde_json::json!("b")));
        assert_eq!(merged["vscode"]["settings"]["a"], 1);
        assert_eq!(merged["vscode"]["settings"]["b"], 1);
        for id in [id_a, id_b] {
            let _ = std::fs::remove_dir_all(feature_cache_dir().join(sanitize_id(id)));
        }
    }

    #[test]
    fn test_collect_feature_lifecycle_hooks() {
        let id = "ghcr.io/test/feature-hooks";
        let dir = feature_cache_dir().join(sanitize_id(id));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("devcontainer-feature.json"),
            r#"{"postCreateCommand": "echo feature", "postStartCommand": ["echo", "start"]}"#,
        )
        .unwrap();
        let features = HashMap::from([(id.to_string(), serde_json::json!({}))]);
        let order = feature_ids_sorted(&Some(features.clone()));
        let hooks = collect_feature_lifecycle_hooks(&features, &order);
        assert_eq!(hooks.len(), 2);
        assert_eq!(hooks[0].0, "postCreateCommand");
        assert_eq!(hooks[0].1, serde_json::json!("echo feature"));
        assert_eq!(hooks[1].0, "postStartCommand");
        assert_eq!(
            collect_feature_lifecycle_hooks(&HashMap::new(), &[]).len(),
            0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_feature_hooks_and_customizations_follow_install_order() {
        let id_a = "ghcr.io/test/order-a";
        let id_b = "ghcr.io/test/order-b";
        for (id, key) in [(id_a, "a"), (id_b, "b")] {
            let dir = feature_cache_dir().join(sanitize_id(id));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("devcontainer-feature.json"),
                format!(
                    r#"{{"postCreateCommand": "echo {key}", "customizations": {{"vscode": {{"x": "{key}"}}}}}}"#
                ),
            )
            .unwrap();
        }
        let features = HashMap::from([
            (id_a.to_string(), serde_json::json!({})),
            (id_b.to_string(), serde_json::json!({})),
        ]);
        let order = vec![id_b.to_string(), id_a.to_string()];
        let hooks = collect_feature_lifecycle_hooks(&features, &order);
        assert_eq!(hooks.len(), 2);
        assert_eq!(hooks[0].1, serde_json::json!("echo b"));
        assert_eq!(hooks[1].1, serde_json::json!("echo a"));
        // Later features in install order override earlier values
        let merged = collect_feature_customizations(&features, &order);
        assert_eq!(merged["vscode"]["x"], "a");
        for id in [id_a, id_b] {
            let _ = std::fs::remove_dir_all(feature_cache_dir().join(sanitize_id(id)));
        }
    }

    #[test]
    fn test_feature_ids_sorted_includes_cached_dependencies() {
        let dep = "ghcr.io/test/restart-dep";
        let main = "ghcr.io/test/restart-main";
        let dep_meta = r#"{"id":"restart-dep","version":"1.0.0","name":"dep"}"#.to_string();
        let main_meta = format!(
            r#"{{"id":"restart-main","version":"1.0.0","name":"main","dependsOn":{{"{dep}":{{}}}}}}"#
        );
        for (id, meta) in [(dep, dep_meta), (main, main_meta)] {
            let dir = feature_cache_dir().join(sanitize_id(id));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("devcontainer-feature.json"), meta).unwrap();
        }
        let features = Some(HashMap::from([(main.to_string(), serde_json::json!({}))]));
        assert_eq!(
            feature_ids_sorted(&features),
            vec![dep.to_string(), main.to_string()]
        );
        // Missing cache metadata still yields the requested id
        let uncached = "ghcr.io/test/restart-uncached";
        let features2 = Some(HashMap::from([(
            uncached.to_string(),
            serde_json::json!({}),
        )]));
        assert_eq!(feature_ids_sorted(&features2), vec![uncached.to_string()]);
        for id in [dep, main] {
            let _ = std::fs::remove_dir_all(feature_cache_dir().join(sanitize_id(id)));
        }
    }

    #[test]
    fn test_invalid_feature_option_values() {
        let dir = std::env::temp_dir().join("bondar-feature-enum-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("devcontainer-feature.json"),
            r#"{"options":{"version":{"type":"string","enum":["18","20"]},"flag":{"type":"boolean","enum":[true,false]}}}"#,
        )
        .unwrap();
        // Valid values produce no messages (boolean compared as string)
        let valid = serde_json::json!({"version": "18", "flag": true});
        assert!(invalid_feature_option_values("ghcr.io/a/b", &valid, &dir).is_empty());
        // Invalid values are reported
        let invalid = serde_json::json!({"version": "19", "flag": "maybe"});
        let messages = invalid_feature_option_values("ghcr.io/a/b", &invalid, &dir);
        assert_eq!(messages.len(), 2);
        assert!(messages[0].contains("version"));
        assert!(messages[1].contains("flag"));
        // Unknown option names are ignored
        let unknown = serde_json::json!({"other": "x"});
        assert!(invalid_feature_option_values("ghcr.io/a/b", &unknown, &dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn test_sort_by_installs_after_empty() {
        let empty: HashMap<String, serde_json::Value> = HashMap::new();
        assert!(sort_by_installs_after(&empty).is_empty());
    }

    #[test]
    fn test_collect_and_apply_feature_container_properties() {
        let dir = std::env::temp_dir().join("bondar-feature-props-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("devcontainer-feature.json"),
            r#"{
                "containerEnv": {"FEATURE_VAR": "fv", "NUM": 1},
                "mounts": ["type=volume,source=fvol,target=/fdata", {"type": "bind", "source": "/host", "target": "/in", "readonly": true}],
                "privileged": true,
                "init": true,
                "capAdd": ["SYS_PTRACE"],
                "securityOpt": ["seccomp=unconfined"]
            }"#,
        )
        .unwrap();
        let props = collect_feature_container_properties("ghcr.io/a/b", &dir);
        assert_eq!(
            props.container_env.get("FEATURE_VAR").map(String::as_str),
            Some("fv")
        );
        assert_eq!(
            props.container_env.get("NUM").map(String::as_str),
            Some("1")
        );
        assert_eq!(props.mounts.len(), 2);
        assert!(props.privileged);
        assert!(props.init);
        assert_eq!(props.cap_add, vec!["SYS_PTRACE".to_string()]);
        assert_eq!(props.security_opt, vec!["seccomp=unconfined".to_string()]);

        // Applying merges without overriding user values
        let mut cfg = crate::config::DevContainerConfig {
            container_env: HashMap::from([("FEATURE_VAR".to_string(), "user".to_string())]),
            cap_add: vec!["NET_ADMIN".to_string()],
            ..Default::default()
        };
        apply_feature_container_properties(&mut cfg, &props);
        assert_eq!(cfg.container_env.get("FEATURE_VAR").unwrap(), "user");
        assert_eq!(cfg.container_env.get("NUM").unwrap(), "1");
        assert_eq!(cfg.mounts.len(), 2);
        assert_eq!(cfg.privileged, Some(true));
        assert_eq!(cfg.init, Some(true));
        assert_eq!(
            cfg.cap_add,
            vec!["NET_ADMIN".to_string(), "SYS_PTRACE".to_string()]
        );
        assert_eq!(cfg.security_opt, vec!["seccomp=unconfined".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_collect_feature_customizations() {
        // Place metadata under the feature cache dir with a unique id
        let id = "ghcr.io/test/feature";
        let dir = feature_cache_dir().join(sanitize_id(id));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("devcontainer-feature.json"),
            r#"{"customizations": {"vscode": {"settings": {"a": 1}}}}"#,
        )
        .unwrap();
        let features = HashMap::from([(id.to_string(), serde_json::json!({}))]);
        let order = feature_ids_sorted(&Some(features.clone()));
        let merged = collect_feature_customizations(&features, &order);
        assert_eq!(merged["vscode"]["settings"]["a"], 1);

        // The string version shorthand resolves to the same cache directory
        let features_str = HashMap::from([(id.to_string(), serde_json::json!("1"))]);
        let order_str = feature_ids_sorted(&Some(features_str.clone()));
        let merged_str = collect_feature_customizations(&features_str, &order_str);
        assert_eq!(merged_str["vscode"]["settings"]["a"], 1);

        // No features -> empty object
        assert!(
            collect_feature_customizations(&HashMap::new(), &[])
                .as_object()
                .unwrap()
                .is_empty()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
