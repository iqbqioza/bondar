use crate::error::Result;
use std::path::Path;

pub fn check_host_requirements(req: &serde_json::Value, _workspace_folder: &Path) -> Result<()> {
    if let Some(cpus) = req.get("cpus") {
        if let Some(c) = cpus.as_u64() {
            if c == 0 {
                eprintln!("Warning: hostRequirements.cpus must be at least 1, got 0");
            }
            let available = available_cpus();
            if available < c {
                eprintln!(
                    "Warning: hostRequirements.cpus {c} required but only {available} available"
                );
            }
        } else {
            eprintln!("Warning: hostRequirements.cpus must be an integer, got {cpus}");
        }
    }

    if let Some(mem) = req.get("memory") {
        let Some(mem_str) = mem.as_str() else {
            eprintln!("Warning: hostRequirements.memory must be a string, got {mem}");
            return Ok(());
        };
        if let Some(required_bytes) = parse_size(mem_str) {
            if let Some(available_bytes) = available_memory_bytes()
                && available_bytes < required_bytes
            {
                eprintln!(
                    "Warning: hostRequirements.memory {mem_str} required but only {} available",
                    format_bytes(available_bytes)
                );
            }
        } else {
            eprintln!("Warning: invalid hostRequirements.memory format: {mem_str}");
        }
    }

    if let Some(storage) = req.get("storage") {
        let Some(storage_str) = storage.as_str() else {
            eprintln!("Warning: hostRequirements.storage must be a string, got {storage}");
            return Ok(());
        };
        if let Some(required_bytes) = parse_size(storage_str) {
            if let Some(available_bytes) = available_storage_bytes(_workspace_folder)
                && available_bytes < required_bytes
            {
                eprintln!(
                    "Warning: hostRequirements.storage {storage_str} required but only {} available",
                    format_bytes(available_bytes)
                );
            }
        } else {
            eprintln!("Warning: invalid hostRequirements.storage format: {storage_str}");
        }
    }

    if let Some(gpu) = req.get("gpu") {
        match gpu {
            serde_json::Value::Bool(true) => {
                if !has_gpu() {
                    eprintln!("Warning: hostRequirements.gpu required but no GPU detected");
                }
            }
            serde_json::Value::String(s) if s == "optional" => {}
            serde_json::Value::Object(map) => {
                if !has_gpu() {
                    eprintln!("Warning: hostRequirements.gpu object required but no GPU detected");
                }
                if let Some(cores) = map.get("cores")
                    && cores.as_u64().is_none()
                {
                    eprintln!(
                        "Warning: hostRequirements.gpu.cores must be a non-negative integer, got {cores}"
                    );
                }
                if let Some(mem_str) = map.get("memory").and_then(|v| v.as_str())
                    && parse_size(mem_str).is_none()
                {
                    eprintln!("Warning: invalid hostRequirements.gpu.memory format: {mem_str}");
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn available_cpus() -> u64 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u64)
        .unwrap_or(1)
}

#[cfg(target_os = "linux")]
fn available_memory_bytes() -> Option<u64> {
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    // Prefer MemAvailable (actual free memory) over MemTotal
    let mut mem_available = None;
    let mut mem_total = None;
    for line in content.lines() {
        if line.starts_with("MemAvailable:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2
                && let Ok(kb) = parts[1].parse::<u64>()
            {
                mem_available = Some(kb * 1024);
            }
        } else if line.starts_with("MemTotal:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2
                && let Ok(kb) = parts[1].parse::<u64>()
            {
                mem_total = Some(kb * 1024);
            }
        }
    }
    mem_available.or(mem_total)
}

#[cfg(target_os = "macos")]
fn available_memory_bytes() -> Option<u64> {
    let output = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn available_memory_bytes() -> Option<u64> {
    None
}

#[cfg(unix)]
fn available_storage_bytes(path: &Path) -> Option<u64> {
    // Use POSIX output (-P) for stable column order across locales
    let output = std::process::Command::new("df")
        .args(["-k", "-P"])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4
            && let Ok(available_kb) = parts[3].parse::<u64>()
        {
            return Some(available_kb * 1024);
        }
    }
    None
}

#[cfg(not(unix))]
fn available_storage_bytes(_path: &Path) -> Option<u64> {
    None
}

fn has_gpu() -> bool {
    std::process::Command::new("nvidia-smi")
        .arg("-L")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        || (cfg!(unix) && Path::new("/dev/nvidia0").exists())
        || (cfg!(unix) && Path::new("/dev/dri").exists())
}

fn parse_size(s: &str) -> Option<u64> {
    let lower = s.trim().to_ascii_lowercase();
    let (num_str, mult) = if lower.ends_with("tb") {
        (&lower[..lower.len() - 2], 1024u64.pow(4))
    } else if lower.ends_with("gb") {
        (&lower[..lower.len() - 2], 1024u64.pow(3))
    } else if lower.ends_with("mb") {
        (&lower[..lower.len() - 2], 1024u64.pow(2))
    } else if lower.ends_with("kb") {
        (&lower[..lower.len() - 2], 1024)
    } else {
        // Unitless values are bytes (the schema pattern allows e.g. "0" or "1024")
        return lower.parse::<u64>().ok();
    };
    let num: f64 = num_str.trim().parse().ok()?;
    Some((num * mult as f64) as u64)
}

fn format_bytes(bytes: u64) -> String {
    const TB: u64 = 1024 * 1024 * 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    const MB: u64 = 1024 * 1024;
    const KB: u64 = 1024;
    if bytes >= TB {
        format!("{:.1}tb", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1}gb", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}mb", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}kb", bytes as f64 / KB as f64)
    } else {
        format!("{bytes}b")
    }
}

pub fn handle_update_remote_user_uid(
    config: &crate::config::DevContainerConfig,
    container_name: &str,
) -> Result<()> {
    let should_update = config.update_remote_user_uid.unwrap_or(true);
    if !should_update {
        return Ok(());
    }

    let target_user = config
        .remote_user
        .as_deref()
        .or(config.container_user.as_deref());

    let Some(user) = target_user else {
        return Ok(());
    };

    if user == "root" {
        return Ok(());
    }

    let host_uid = get_host_uid();
    let host_gid = get_host_gid();

    // On Windows there is no POSIX UID/GID concept; on Unix running as root
    // makes UID mapping pointless (root can access any file).
    if host_uid == 0 {
        if cfg!(windows) {
            eprintln!(
                "Warning: UID/GID synchronization is not supported on Windows; skipping updateRemoteUserUID"
            );
        } else {
            eprintln!(
                "Warning: bondar is running as root; skipping updateRemoteUserUID (container user would map to uid 0)"
            );
        }
        return Ok(());
    }

    println!("Updating UID/GID for user {user} to {host_uid}:{host_gid} (updateRemoteUserUID)");

    let check_user = std::process::Command::new("docker")
        .args(["exec", container_name, "id", user])
        .output();

    // A spawn failure (docker unavailable) must not trigger user creation
    let Ok(check_user) = check_user else {
        eprintln!(
            "Warning: failed to run 'docker exec id {user}' (is the docker daemon running?); skipping updateRemoteUserUID"
        );
        return Ok(());
    };
    let user_exists = check_user.status.success();

    if check_user.status.success() {
        let stdout = String::from_utf8_lossy(&check_user.stdout);
        // stdout like uid=1000(vscode) gid=1000(vscode)
        let current_uid = parse_id_output(&stdout, "uid=");
        let current_gid = parse_id_output(&stdout, "gid=");
        if current_uid == Some(host_uid) && current_gid == Some(host_gid) {
            return Ok(());
        }
        if current_uid != Some(host_uid) {
            eprintln!("Updating UID for {user} from {current_uid:?} to {host_uid}");
        }
        if current_gid != Some(host_gid) {
            eprintln!("Updating GID for {user} from {current_gid:?} to {host_gid}");
        }
    } else {
        eprintln!("Warning: user '{user}' not found in container; will attempt to create it");
    }

    if user_exists {
        let usermod = std::process::Command::new("docker")
            .args([
                "exec",
                "--user",
                "root",
                container_name,
                "usermod",
                "-u",
                &host_uid.to_string(),
                user,
            ])
            .output();

        match usermod {
            Ok(o) if o.status.success() => {
                println!("Updated UID for {user}");
            }
            Ok(o) => {
                eprintln!(
                    "Warning: usermod failed for {user}: {}",
                    String::from_utf8_lossy(&o.stderr)
                );
            }
            Err(e) => {
                eprintln!("Warning: failed to run usermod: {e}");
            }
        }

        // Resolve the user's primary group name before groupmod, since the group
        // may not share the user's name (e.g. user "node" with group "node" vs "users").
        let group_target = resolve_primary_group(container_name, user);

        let groupmod = std::process::Command::new("docker")
            .args([
                "exec",
                "--user",
                "root",
                container_name,
                "groupmod",
                "-g",
                &host_gid.to_string(),
                &group_target,
            ])
            .output();

        match groupmod {
            Ok(o) if o.status.success() => {
                println!("Updated GID for {group_target}");
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                if !stderr.to_lowercase().contains("no such") {
                    eprintln!("Warning: groupmod failed for {group_target}: {stderr}");
                }
            }
            Err(e) => {
                eprintln!("Warning: failed to run groupmod: {e}");
            }
        }

        // Chown workspace if it exists inside container
        chown_workspace(config, container_name, user);
    }

    // Fallback: create the user (and its group) when it does not exist in the container
    if !user_exists {
        eprintln!("Attempting to create user {user} with UID {host_uid}");
        // Create the group first (fails harmlessly if it already exists)
        let _ = std::process::Command::new("docker")
            .args([
                "exec",
                "--user",
                "root",
                container_name,
                "groupadd",
                "-g",
                &host_gid.to_string(),
                user,
            ])
            .status();
        let useradd = std::process::Command::new("docker")
            .args([
                "exec",
                "--user",
                "root",
                container_name,
                "useradd",
                "-m",
                "-u",
                &host_uid.to_string(),
                "-g",
                user,
                user,
            ])
            .output();
        if let Ok(ua) = useradd {
            if ua.status.success() {
                println!("Created user {user}");
                // After creating the user, take ownership of the workspace
                chown_workspace(config, container_name, user);
            } else {
                eprintln!(
                    "Warning: useradd failed: {}",
                    String::from_utf8_lossy(&ua.stderr)
                );
            }
        }
    }

    Ok(())
}

pub fn probe_user_env(
    container_name: &str,
    user: Option<&str>,
    probe: &str,
) -> Option<std::collections::HashMap<String, String>> {
    if probe == "none" {
        return None;
    }
    let (shell, args) = match probe {
        "interactiveShell" => ("bash", vec!["-i", "-c", "env"]),
        "loginShell" => ("bash", vec!["-l", "-c", "env"]),
        "loginInteractiveShell" => ("bash", vec!["-l", "-i", "-c", "env"]),
        _ => ("sh", vec!["-c", "env"]),
    };
    let mut cmd = std::process::Command::new("docker");
    cmd.arg("exec");
    if let Some(u) = user {
        cmd.arg("--user").arg(u);
    }
    cmd.arg(container_name).arg(shell);
    for a in args {
        cmd.arg(a);
    }
    let output = cmd.output().ok()?;
    if output.status.success() {
        return parse_env_output(&output.stdout);
    }
    // Fall back to `sh -c env` when the requested shell (e.g. bash) is missing
    let mut fallback = std::process::Command::new("docker");
    fallback.arg("exec");
    if let Some(u) = user {
        fallback.arg("--user").arg(u);
    }
    fallback.arg(container_name).arg("sh").arg("-c").arg("env");
    let fb = fallback.output().ok()?;
    if !fb.status.success() {
        return None;
    }
    parse_env_output(&fb.stdout)
}

fn parse_env_output(stdout: &[u8]) -> Option<std::collections::HashMap<String, String>> {
    let stdout = String::from_utf8_lossy(stdout);
    let mut env = std::collections::HashMap::new();
    for line in stdout.lines() {
        if let Some((k, v)) = line.split_once('=') {
            env.insert(k.to_string(), v.to_string());
        }
    }
    if env.is_empty() { None } else { Some(env) }
}

#[cfg(unix)]
fn get_host_uid() -> u32 {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(1000)
}

#[cfg(not(unix))]
fn get_host_uid() -> u32 {
    0
}

/// Chown the workspace directory to the given user, guarding against "/".
fn chown_workspace(config: &crate::config::DevContainerConfig, container_name: &str, user: &str) {
    // For compose, the default workspace folder is "/" (spec); chown of "/"
    // is skipped below, so only chown when workspaceFolder is set explicitly.
    let chown_target = if config.docker_compose_file.is_some() {
        config
            .workspace_folder
            .clone()
            .unwrap_or_else(|| "/".to_string())
    } else {
        config.workspace_folder_or_default()
    };
    if chown_target == "/" {
        eprintln!(
            "Warning: skipping chown of '/' (would alter container-wide ownership); set workspaceFolder to a specific directory"
        );
        return;
    }
    // Use the user's primary group name (may differ from the user name)
    let group = resolve_primary_group(container_name, user);
    let chown = std::process::Command::new("docker")
        .args([
            "exec",
            "--user",
            "root",
            container_name,
            "chown",
            "-R",
            &format!("{user}:{group}"),
            &chown_target,
        ])
        .output();
    if let Ok(o) = chown {
        if o.status.success() {
            println!("Chowned {chown_target} to {user}");
        } else {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if !stderr.to_lowercase().contains("no such") {
                eprintln!("Warning: chown failed: {stderr}");
            }
        }
    }
}

fn resolve_primary_group(container_name: &str, user: &str) -> String {
    std::process::Command::new("docker")
        .args(["exec", container_name, "id", "-g", "-n", user])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| user.to_string())
}

#[cfg(unix)]
fn get_host_gid() -> u32 {
    std::process::Command::new("id")
        .arg("-g")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(1000)
}

#[cfg(not(unix))]
fn get_host_gid() -> u32 {
    0
}

fn parse_id_output(output: &str, prefix: &str) -> Option<u32> {
    let start = output.find(prefix)?;
    let after = &output[start + prefix.len()..];
    let end = after
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(after.len());
    after[..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size() {
        assert_eq!(parse_size("4gb"), Some(4 * 1024 * 1024 * 1024));
        assert_eq!(parse_size("512mb"), Some(512 * 1024 * 1024));
        assert_eq!(
            parse_size("1.5gb"),
            Some((1.5 * 1024.0 * 1024.0 * 1024.0) as u64)
        );
        assert_eq!(parse_size("invalid"), None);
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0gb");
    }

    #[test]
    fn test_parse_id() {
        assert_eq!(
            parse_id_output("uid=1000(vscode) gid=1000", "uid="),
            Some(1000)
        );
        assert_eq!(parse_id_output("uid=0(root) gid=0", "gid="), Some(0));
    }

    #[test]
    fn test_available_cpus() {
        assert!(available_cpus() >= 1);
    }

    #[test]
    fn test_format_bytes_boundaries() {
        assert_eq!(format_bytes(1023), "1023b");
        assert_eq!(format_bytes(1024), "1.0kb");
        assert_eq!(format_bytes(1024 * 1024), "1.0mb");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024), "5.0gb");
        assert_eq!(format_bytes(2 * 1024u64.pow(4)), "2.0tb");
        assert_eq!(format_bytes(0), "0b");
    }

    #[test]
    fn test_parse_size_case_insensitive() {
        assert_eq!(parse_size("4GB"), parse_size("4gb"));
        assert_eq!(parse_size("512MB"), Some(512 * 1024 * 1024));
        assert_eq!(parse_size("2TB"), Some(2 * 1024u64.pow(4)));
    }

    #[test]
    fn test_parse_size_edge() {
        assert_eq!(
            parse_size("0.5gb"),
            Some((0.5 * 1024.0 * 1024.0 * 1024.0) as u64)
        );
        assert_eq!(parse_size("1kb"), Some(1024));
        assert_eq!(parse_size("10b"), None);
        assert_eq!(parse_size("gb"), None);
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("1.5"), None);
        // Unitless values are bytes (schema pattern allows them)
        assert_eq!(parse_size("0"), Some(0));
        assert_eq!(parse_size("1024"), Some(1024));
        assert_eq!(parse_size("2TB"), Some(2 * 1024u64.pow(4)));
    }

    #[test]
    fn test_parse_id_output_empty() {
        assert_eq!(parse_id_output("", "uid="), None);
        assert_eq!(parse_id_output("uid=", "uid="), None);
        assert_eq!(parse_id_output("gid=123", "uid="), None);
    }

    #[cfg(unix)]
    #[test]
    fn test_get_host_uid() {
        assert!(get_host_uid() > 0);
    }

    #[test]
    fn test_check_host_requirements_no_req() {
        // No requirements -> always Ok (warnings only)
        assert!(check_host_requirements(&serde_json::json!({}), Path::new(".")).is_ok());
        assert!(check_host_requirements(&serde_json::json!({"cpus": 1}), Path::new(".")).is_ok());
    }
}
