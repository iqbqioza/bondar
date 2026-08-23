use crate::error::Result;
use std::path::Path;

pub fn check_host_requirements(req: &serde_json::Value, _workspace_folder: &Path) -> Result<()> {
    if let Some(cpus) = req.get("cpus").and_then(|v| v.as_u64()) {
        let available = available_cpus();
        if available < cpus {
            eprintln!(
                "Warning: hostRequirements.cpus {cpus} required but only {available} available"
            );
        }
    }

    if let Some(mem_str) = req.get("memory").and_then(|v| v.as_str()) {
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

    if let Some(storage_str) = req.get("storage").and_then(|v| v.as_str()) {
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
            serde_json::Value::Object(_) if !has_gpu() => {
                eprintln!("Warning: hostRequirements.gpu object required but no GPU detected");
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

fn available_memory_bytes() -> Option<u64> {
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2
                && let Ok(kb) = parts[1].parse::<u64>()
            {
                return Some(kb * 1024);
            }
        }
    }
    None
}

fn available_storage_bytes(path: &Path) -> Option<u64> {
    let output = std::process::Command::new("df")
        .arg("-k")
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

fn has_gpu() -> bool {
    std::process::Command::new("nvidia-smi")
        .arg("-L")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        || Path::new("/dev/nvidia0").exists()
}

fn parse_size(s: &str) -> Option<u64> {
    let lower = s.to_ascii_lowercase();
    let (num_str, mult) = if lower.ends_with("tb") {
        (&lower[..lower.len() - 2], 1024u64.pow(4))
    } else if lower.ends_with("gb") {
        (&lower[..lower.len() - 2], 1024u64.pow(3))
    } else if lower.ends_with("mb") {
        (&lower[..lower.len() - 2], 1024u64.pow(2))
    } else if lower.ends_with("kb") {
        (&lower[..lower.len() - 2], 1024)
    } else {
        return None;
    };
    let num: f64 = num_str.trim().parse().ok()?;
    Some((num * mult as f64) as u64)
}

fn format_bytes(bytes: u64) -> String {
    const GB: u64 = 1024 * 1024 * 1024;
    const MB: u64 = 1024 * 1024;
    const KB: u64 = 1024;
    if bytes >= GB {
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

    println!("Updating UID/GID for user {user} to {host_uid}:{host_gid} (updateRemoteUserUID)");

    let check_user = std::process::Command::new("docker")
        .args(["exec", container_name, "id", user])
        .output();

    if let Ok(output) = check_user {
        if !output.status.success() {
            eprintln!("Warning: user {user} not found in container, skipping UID update");
            return Ok(());
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        // stdout like uid=1000(vscode) gid=1000(vscode)
        if let Some(current_uid) = parse_id_output(&stdout, "uid=")
            && current_uid == host_uid
        {
            return Ok(());
        }
    }

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

    let groupmod = std::process::Command::new("docker")
        .args([
            "exec",
            "--user",
            "root",
            container_name,
            "groupmod",
            "-g",
            &host_gid.to_string(),
            user,
        ])
        .output();

    match groupmod {
        Ok(o) if o.status.success() => {
            println!("Updated GID for {user}");
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if !stderr.contains("no such") {
                eprintln!("Warning: groupmod failed for {user}: {stderr}");
            }
        }
        Err(e) => {
            eprintln!("Warning: failed to run groupmod: {e}");
        }
    }

    Ok(())
}

fn get_host_uid() -> u32 {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(1000)
}

fn get_host_gid() -> u32 {
    std::process::Command::new("id")
        .arg("-g")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(1000)
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
}
