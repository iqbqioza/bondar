use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_bondar");

/// Same FNV-1a hash bondar uses for per-workspace compose projects/image names.
fn workspace_hash8(ws: &std::path::Path) -> String {
    let ws_str = ws.to_string_lossy().to_string();
    let mut hash: u64 = 14695981039346656037;
    for b in ws_str.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("{hash:016x}")[..8].to_string()
}

/// Same FNV-1a hash bondar uses for the per-workspace compose project name.
fn project_name_for(ws: &std::path::Path) -> String {
    format!("bondar-{}", workspace_hash8(ws))
}

fn docker_available() -> bool {
    Command::new("docker")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn make_workspace(name: &str, devcontainer_json: &str) -> PathBuf {
    let ws = std::env::temp_dir().join(format!("bondar-int-{name}"));
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join(".devcontainer")).unwrap();
    std::fs::write(
        ws.join(".devcontainer/devcontainer.json"),
        devcontainer_json,
    )
    .unwrap();
    ws
}

fn cleanup(ws: &PathBuf) {
    let _ = std::fs::remove_dir_all(ws);
}

fn bondar(args: &[&str]) -> std::process::Output {
    Command::new(BIN).args(args).output().expect("run bondar")
}

#[test]
fn test_image_roundtrip() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "image",
        r#"{"name": "int-image", "image": "ubuntu:22.04", "workspaceFolder": "/workspace"}"#,
    );
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up.status.success(),
        "up failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );

    let exec = bondar(&[
        "exec",
        "--workspace-folder",
        ws_str,
        "--",
        "sh",
        "-c",
        "echo roundtrip-ok",
    ]);
    assert!(
        exec.status.success(),
        "exec failed: {}",
        String::from_utf8_lossy(&exec.stderr)
    );
    assert!(String::from_utf8_lossy(&exec.stdout).contains("roundtrip-ok"));

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(
        down.status.success(),
        "down failed: {}",
        String::from_utf8_lossy(&down.stderr)
    );

    cleanup(&ws);
}

#[test]
fn test_build_roundtrip() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "build",
        r#"{"name": "int-build", "build": {"dockerfile": "Dockerfile"}, "workspaceFolder": "/workspace"}"#,
    );
    std::fs::write(
        ws.join(".devcontainer/Dockerfile"),
        "FROM ubuntu:22.04\nRUN echo built > /tmp/built.txt\n",
    )
    .unwrap();
    let ws_str = ws.to_str().unwrap();

    let build = bondar(&["build", "--workspace-folder", ws_str]);
    assert!(
        build.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up.status.success(),
        "up failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );

    let exec = bondar(&[
        "exec",
        "--workspace-folder",
        ws_str,
        "--",
        "sh",
        "-c",
        "cat /tmp/built.txt",
    ]);
    assert!(
        exec.status.success(),
        "exec failed: {}",
        String::from_utf8_lossy(&exec.stderr)
    );
    assert!(String::from_utf8_lossy(&exec.stdout).contains("built"));

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(down.status.success());
    let _ = Command::new("docker")
        .args([
            "rmi",
            "-f",
            &format!("bondar-int-build-{}", workspace_hash8(&ws)),
        ])
        .output();

    cleanup(&ws);
}

#[test]
fn test_compose_roundtrip() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = std::env::temp_dir().join("bondar-int-compose");
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join(".devcontainer")).unwrap();
    std::fs::write(
        ws.join("docker-compose.yml"),
        "services:\n  app:\n    image: ubuntu:22.04\n    command: sh -c 'while sleep 1000; do :; done'\n    volumes:\n      - .:/workspace\n",
    )
    .unwrap();
    std::fs::write(
        ws.join(".devcontainer/devcontainer.json"),
        r#"{"name": "int-compose", "dockerComposeFile": "../docker-compose.yml", "service": "app", "workspaceFolder": "/workspace", "remoteEnv": {"WS": "${containerWorkspaceFolder}"}}"#,
    )
    .unwrap();
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up.status.success(),
        "compose up failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );

    let exec = bondar(&[
        "exec",
        "--workspace-folder",
        ws_str,
        "--",
        "sh",
        "-c",
        "echo compose-ok",
    ]);
    assert!(
        exec.status.success(),
        "compose exec failed: {}",
        String::from_utf8_lossy(&exec.stderr)
    );
    assert!(String::from_utf8_lossy(&exec.stdout).contains("compose-ok"));

    // `--workdir` must not change what ${containerWorkspaceFolder} expands to
    let exec_workdir = bondar(&[
        "exec",
        "--workspace-folder",
        ws_str,
        "--workdir",
        "/tmp",
        "--",
        "sh",
        "-c",
        "pwd && echo \"WS=$WS\"",
    ]);
    assert!(
        exec_workdir.status.success(),
        "compose exec --workdir failed: {}",
        String::from_utf8_lossy(&exec_workdir.stderr)
    );
    let stdout = String::from_utf8_lossy(&exec_workdir.stdout);
    assert!(stdout.contains("/tmp"), "workdir not applied: {stdout}");
    assert!(
        stdout.contains("WS=/workspace"),
        "containerWorkspaceFolder expanded from --workdir: {stdout}"
    );

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(
        down.status.success(),
        "compose down failed: {}",
        String::from_utf8_lossy(&down.stderr)
    );

    cleanup(&ws);
}

#[test]
fn test_update_remote_user_uid() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "uid",
        r#"{"name": "int-uid", "image": "ubuntu:22.04", "workspaceFolder": "/workspace", "remoteUser": "vscode", "userEnvProbe": "none"}"#,
    );
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up.status.success(),
        "up failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );

    // The vscode user should have been created by updateRemoteUserUID
    let exec = bondar(&["exec", "--workspace-folder", ws_str, "--", "id", "vscode"]);
    assert!(
        exec.status.success(),
        "id vscode failed: {}",
        String::from_utf8_lossy(&exec.stderr)
    );
    assert!(String::from_utf8_lossy(&exec.stdout).contains("vscode"));

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(down.status.success());
    cleanup(&ws);
}

#[test]
fn test_user_env_probe() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "probe",
        r#"{"name": "int-probe", "image": "ubuntu:22.04", "workspaceFolder": "/workspace", "userEnvProbe": "interactiveShell"}"#,
    );
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up.status.success(),
        "up failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );
    assert!(String::from_utf8_lossy(&up.stdout).contains("Probed"));

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(down.status.success());
    cleanup(&ws);
}

#[test]
fn test_read_configuration() {
    let ws = make_workspace(
        "readcfg",
        r#"{"name": "int-read", "image": "ubuntu:22.04", "workspaceFolder": "/workspace"}"#,
    );
    let ws_str = ws.to_str().unwrap();

    let valid = bondar(&["read-configuration", "--workspace-folder", ws_str]);
    assert!(
        valid.status.success(),
        "valid config rejected: {}",
        String::from_utf8_lossy(&valid.stderr)
    );
    assert!(String::from_utf8_lossy(&valid.stdout).contains("valid"));

    // A minimal image config without workspaceFolder must also be valid
    std::fs::write(
        ws.join(".devcontainer/devcontainer.json"),
        r#"{"image": "ubuntu:22.04"}"#,
    )
    .unwrap();
    let minimal = bondar(&["read-configuration", "--workspace-folder", ws_str]);
    assert!(
        minimal.status.success(),
        "minimal config rejected: {}",
        String::from_utf8_lossy(&minimal.stderr)
    );

    // portsAttributes protocol "udp" is supported by bondar
    std::fs::write(
        ws.join(".devcontainer/devcontainer.json"),
        r#"{"image": "ubuntu:22.04", "portsAttributes": {"9090": {"protocol": "udp"}}}"#,
    )
    .unwrap();
    let udp = bondar(&["read-configuration", "--workspace-folder", ws_str]);
    assert!(
        udp.status.success(),
        "udp protocol rejected: {}",
        String::from_utf8_lossy(&udp.stderr)
    );

    // Compose configurations default the workspace folder to "/" and do not
    // require an explicit workspaceFolder (matching the reference CLI)
    std::fs::write(
        ws.join(".devcontainer/devcontainer.json"),
        r#"{"dockerComposeFile": "docker-compose.yml", "service": "app"}"#,
    )
    .unwrap();
    let compose_default = bondar(&["read-configuration", "--workspace-folder", ws_str]);
    assert!(
        compose_default.status.success(),
        "compose without workspaceFolder rejected: {}",
        String::from_utf8_lossy(&compose_default.stderr)
    );

    // tmpfs object mounts are supported by bondar (docker --mount)
    std::fs::write(
        ws.join(".devcontainer/devcontainer.json"),
        r#"{"image": "ubuntu:22.04", "mounts": [{"type": "tmpfs", "target": "/tmp-data"}]}"#,
    )
    .unwrap();
    let tmpfs = bondar(&["read-configuration", "--workspace-folder", ws_str]);
    assert!(
        tmpfs.status.success(),
        "tmpfs mount rejected: {}",
        String::from_utf8_lossy(&tmpfs.stderr)
    );

    // Invalid config (waitFor out of enum) -> exit 1
    std::fs::write(
        ws.join(".devcontainer/devcontainer.json"),
        r#"{"image": "ubuntu:22.04", "waitFor": "bogus"}"#,
    )
    .unwrap();
    let invalid = bondar(&["read-configuration", "--workspace-folder", ws_str]);
    assert!(!invalid.status.success(), "invalid config should fail");
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("INVALID"));

    cleanup(&ws);
}

#[test]
fn test_logs() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "logs",
        r#"{"name": "int-logs", "image": "ubuntu:22.04", "workspaceFolder": "/workspace"}"#,
    );
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(up.status.success());

    let logs = bondar(&["logs", "--workspace-folder", ws_str]);
    assert!(
        logs.status.success(),
        "logs failed: {}",
        String::from_utf8_lossy(&logs.stderr)
    );

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(down.status.success());
    cleanup(&ws);
}

#[test]
fn test_wait_for_background() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "waitfor",
        r#"{"name": "int-waitfor", "image": "ubuntu:22.04", "workspaceFolder": "/workspace", "waitFor": "initializeCommand", "onCreateCommand": "echo created > /tmp/oc.txt", "postAttachCommand": "echo attached > /tmp/pa.txt", "userEnvProbe": "none"}"#,
    );
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up.status.success(),
        "up failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );
    assert!(String::from_utf8_lossy(&up.stdout).contains("in background"));

    // Poll instead of a fixed sleep so slow CI does not flake
    let mut ok = false;
    for _ in 0..50 {
        let exec = bondar(&[
            "exec",
            "--workspace-folder",
            ws_str,
            "--",
            "cat",
            "/tmp/oc.txt",
        ]);
        if exec.status.success() && String::from_utf8_lossy(&exec.stdout).contains("created") {
            ok = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(ok, "background onCreate did not run in time");

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(down.status.success());
    cleanup(&ws);
}

#[test]
fn test_shell_command() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "shell",
        r#"{"name": "int-shell", "image": "ubuntu:22.04", "workspaceFolder": "/workspace", "userEnvProbe": "none"}"#,
    );
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(up.status.success());

    // Non-interactive shell: runs `sh -c ...` and exits (status may vary on TTY)
    let _ = bondar(&["shell", "--workspace-folder", ws_str]);

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(down.status.success());
    cleanup(&ws);
}

#[test]
fn test_remove_existing_container() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "recreate",
        r#"{"name": "int-recreate", "image": "ubuntu:22.04", "workspaceFolder": "/workspace", "onCreateCommand": "echo c > /tmp/c.txt", "userEnvProbe": "none"}"#,
    );
    let ws_str = ws.to_str().unwrap();

    let up1 = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(up1.status.success());

    let up2 = bondar(&[
        "up",
        "--workspace-folder",
        ws_str,
        "--remove-existing-container",
    ]);
    assert!(
        up2.status.success(),
        "recreate failed: {}",
        String::from_utf8_lossy(&up2.stderr)
    );
    assert!(String::from_utf8_lossy(&up2.stdout).contains("onCreateCommand"));

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(down.status.success());
    cleanup(&ws);
}

#[test]
fn test_shutdown_action_stop() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "shutdown",
        r#"{"name": "int-shutdown", "image": "ubuntu:22.04", "workspaceFolder": "/workspace", "shutdownAction": "stopContainer", "userEnvProbe": "none"}"#,
    );
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(up.status.success());

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(
        down.status.success(),
        "down failed: {}",
        String::from_utf8_lossy(&down.stderr)
    );
    assert!(String::from_utf8_lossy(&down.stdout).contains("stopped"));

    let _ = Command::new("docker")
        .args(["rm", "-f", "bondar-int-shutdown"])
        .output();
    cleanup(&ws);
}

#[test]
fn test_compose_stop_action() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = std::env::temp_dir().join("bondar-int-compose-stop");
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join(".devcontainer")).unwrap();
    std::fs::write(
        ws.join("docker-compose.yml"),
        "services:\n  app:\n    image: ubuntu:22.04\n    command: sh -c 'while sleep 1000; do :; done'\n    volumes:\n      - .:/workspace\n",
    )
    .unwrap();
    std::fs::write(
        ws.join(".devcontainer/devcontainer.json"),
        r#"{"name": "int-compose-stop", "dockerComposeFile": "../docker-compose.yml", "service": "app", "workspaceFolder": "/workspace", "shutdownAction": "stopCompose", "userEnvProbe": "none"}"#,
    )
    .unwrap();
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(up.status.success());

    // stopCompose -> `docker compose stop` (container kept)
    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(
        down.status.success(),
        "down failed: {}",
        String::from_utf8_lossy(&down.stderr)
    );
    assert!(String::from_utf8_lossy(&down.stdout).contains("compose stop"));

    // Cleanup leftover compose project (same project name bondar uses)
    let ws_str = ws.join("docker-compose.yml").to_str().unwrap().to_string();
    let project = project_name_for(&ws);
    let _ = Command::new("docker")
        .args(["compose", "--project-name", &project, "-f", &ws_str, "down"])
        .current_dir(&ws)
        .output();
    cleanup(&ws);
}

#[test]
fn test_exec_with_user_and_workdir() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "execopts",
        r#"{"name": "int-execopts", "image": "ubuntu:22.04", "workspaceFolder": "/workspace", "remoteEnv": {"WS": "${containerWorkspaceFolder}", "WB": "${containerWorkspaceFolderBasename}"}, "userEnvProbe": "none"}"#,
    );
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(up.status.success());

    // `--workdir` must not change what ${containerWorkspaceFolder} expands to
    let exec = bondar(&[
        "exec",
        "--workspace-folder",
        ws_str,
        "--user",
        "root",
        "--workdir",
        "/tmp",
        "--",
        "sh",
        "-c",
        "pwd && echo \"WS=$WS WB=$WB\"",
    ]);
    assert!(
        exec.status.success(),
        "exec failed: {}",
        String::from_utf8_lossy(&exec.stderr)
    );
    let stdout = String::from_utf8_lossy(&exec.stdout);
    assert!(stdout.contains("/tmp"), "workdir not applied: {stdout}");
    assert!(
        stdout.contains("WS=/workspace WB=workspace"),
        "containerWorkspaceFolder expanded from --workdir: {stdout}"
    );

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(down.status.success());
    cleanup(&ws);
}

#[test]
fn test_build_no_cache() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "nocache",
        r#"{"name": "int-nocache", "build": {"dockerfile": "Dockerfile"}, "workspaceFolder": "/workspace"}"#,
    );
    std::fs::write(ws.join(".devcontainer/Dockerfile"), "FROM ubuntu:22.04\n").unwrap();
    let ws_str = ws.to_str().unwrap();

    let build = bondar(&["build", "--workspace-folder", ws_str, "--no-cache"]);
    assert!(
        build.status.success(),
        "build --no-cache failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let _ = Command::new("docker")
        .args([
            "rmi",
            "-f",
            &format!("bondar-int-nocache-{}", workspace_hash8(&ws)),
        ])
        .output();
    cleanup(&ws);
}

#[test]
fn test_compose_run_services_includes_primary() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = std::env::temp_dir().join("bondar-int-runsvc");
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join(".devcontainer")).unwrap();
    std::fs::write(
        ws.join("docker-compose.yml"),
        "services:\n  app:\n    image: ubuntu:22.04\n    command: sh -c 'while sleep 1000; do :; done'\n    volumes:\n      - .:/workspace\n  db:\n    image: ubuntu:22.04\n    command: sh -c 'while sleep 1000; do :; done'\n",
    )
    .unwrap();
    std::fs::write(
        ws.join(".devcontainer/devcontainer.json"),
        r#"{"name": "int-runsvc", "dockerComposeFile": "../docker-compose.yml", "service": "app", "runServices": ["db"], "workspaceFolder": "/workspace", "userEnvProbe": "none"}"#,
    )
    .unwrap();
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up.status.success(),
        "compose up with runServices failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );

    // The primary service must be running even though runServices only lists "db"
    let exec = bondar(&[
        "exec",
        "--workspace-folder",
        ws_str,
        "--",
        "sh",
        "-c",
        "echo primary-ok",
    ]);
    assert!(
        exec.status.success(),
        "primary service did not start: {}",
        String::from_utf8_lossy(&exec.stderr)
    );
    assert!(String::from_utf8_lossy(&exec.stdout).contains("primary-ok"));

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(down.status.success());
    cleanup(&ws);
}

#[test]
fn test_same_basename_workspace_isolation() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    // Two workspaces with the same basename must never share containers
    let base_a = std::env::temp_dir().join("bondar-int-collide-a");
    let base_b = std::env::temp_dir().join("bondar-int-collide-b");
    let _ = std::fs::remove_dir_all(&base_a);
    let _ = std::fs::remove_dir_all(&base_b);
    std::fs::create_dir_all(&base_a).unwrap();
    std::fs::create_dir_all(&base_b).unwrap();
    let ws_a = base_a.join("collide");
    let ws_b = base_b.join("collide");
    for ws in [&ws_a, &ws_b] {
        std::fs::create_dir_all(ws.join(".devcontainer")).unwrap();
        std::fs::write(
            ws.join(".devcontainer/devcontainer.json"),
            r#"{"name": "collide", "image": "ubuntu:22.04", "workspaceFolder": "/workspace", "userEnvProbe": "none"}"#,
        )
        .unwrap();
    }

    let up_a = bondar(&["up", "--workspace-folder", ws_a.to_str().unwrap()]);
    assert!(up_a.status.success(), "first up failed");

    // up on the second workspace must refuse to touch the first one's container
    let up_b = bondar(&["up", "--workspace-folder", ws_b.to_str().unwrap()]);
    assert!(
        !up_b.status.success(),
        "second up must fail on a name collision"
    );
    assert!(String::from_utf8_lossy(&up_b.stderr).contains("already exists for workspace"));

    // down on the second workspace must not remove the first one's container
    let down_b = bondar(&["down", "--workspace-folder", ws_b.to_str().unwrap()]);
    assert!(
        !down_b.status.success(),
        "second down must fail on a name collision"
    );

    // The first workspace's container is still there and usable
    let exec_a = bondar(&[
        "exec",
        "--workspace-folder",
        ws_a.to_str().unwrap(),
        "--",
        "sh",
        "-c",
        "echo intact",
    ]);
    assert!(exec_a.status.success());
    assert!(String::from_utf8_lossy(&exec_a.stdout).contains("intact"));

    let down_a = bondar(&["down", "--workspace-folder", ws_a.to_str().unwrap()]);
    assert!(down_a.status.success());
    let _ = std::fs::remove_dir_all(&base_a);
    let _ = std::fs::remove_dir_all(&base_b);
}

#[test]
fn test_read_configuration_merged() {
    let ws = make_workspace(
        "merged",
        r#"{"name": "int-merged", "image": "ubuntu:22.04", "workspaceFolder": "/workspace", "containerEnv": {"FOO": "bar"}}"#,
    );
    let ws_str = ws.to_str().unwrap();

    let out = bondar(&[
        "read-configuration",
        "--workspace-folder",
        ws_str,
        "--include-merged-configuration",
    ]);
    assert!(
        out.status.success(),
        "merged failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Merged configuration"));
    assert!(String::from_utf8_lossy(&out.stdout).contains("FOO"));

    cleanup(&ws);
}

#[test]
fn test_compose_restart_skips_create_lifecycle() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = std::env::temp_dir().join("bondar-int-compose-restart");
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join(".devcontainer")).unwrap();
    std::fs::write(
        ws.join("docker-compose.yml"),
        "services:\n  app:\n    image: ubuntu:22.04\n    command: sh -c 'while sleep 1000; do :; done'\n    volumes:\n      - .:/workspace\n",
    )
    .unwrap();
    std::fs::write(
        ws.join(".devcontainer/devcontainer.json"),
        r#"{"name": "int-compose-restart", "dockerComposeFile": "../docker-compose.yml", "service": "app", "workspaceFolder": "/workspace", "onCreateCommand": "sh -c 'echo run >> /tmp/oc-count.txt'", "userEnvProbe": "none"}"#,
    )
    .unwrap();
    let ws_str = ws.to_str().unwrap();

    let up1 = bondar(&[
        "up",
        "--workspace-folder",
        ws_str,
        "--remove-existing-container",
    ]);
    assert!(
        up1.status.success(),
        "first up failed: {}",
        String::from_utf8_lossy(&up1.stderr)
    );
    assert!(String::from_utf8_lossy(&up1.stdout).contains("Running onCreateCommand"));

    // Stop the service container externally, then `up` must restart it without
    // re-running the create-time lifecycle.
    let project = project_name_for(&ws);
    let ps = Command::new("docker")
        .args([
            "ps",
            "-a",
            "-q",
            "--filter",
            &format!("label=com.docker.compose.project={project}"),
        ])
        .output()
        .unwrap();
    let id = String::from_utf8_lossy(&ps.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .to_string();
    assert!(!id.is_empty(), "service container not found");
    assert!(
        Command::new("docker")
            .args(["stop", &id])
            .status()
            .unwrap()
            .success()
    );

    let up2 = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up2.status.success(),
        "second up failed: {}",
        String::from_utf8_lossy(&up2.stderr)
    );
    let stdout2 = String::from_utf8_lossy(&up2.stdout);
    assert!(
        !stdout2.contains("Running onCreateCommand"),
        "onCreateCommand re-ran on restart: {stdout2}"
    );

    // The lifecycle file must contain exactly one line
    let count = Command::new("docker")
        .args(["exec", &id, "cat", "/tmp/oc-count.txt"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&count.stdout).lines().count(), 1);

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(down.status.success());
    cleanup(&ws);
}

#[test]
fn test_start_existing_container() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "restart",
        r#"{"name": "int-restart", "image": "ubuntu:22.04", "workspaceFolder": "/workspace", "postStartCommand": "echo started > /tmp/ps.txt", "userEnvProbe": "none"}"#,
    );
    let ws_str = ws.to_str().unwrap();

    let up1 = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(up1.status.success());

    // Stop the container externally, then `bondar up` should start it again
    let stop = Command::new("docker")
        .args(["stop", "bondar-int-restart"])
        .output()
        .unwrap();
    assert!(stop.status.success());

    let up2 = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up2.status.success(),
        "restart up failed: {}",
        String::from_utf8_lossy(&up2.stderr)
    );
    assert!(String::from_utf8_lossy(&up2.stdout).contains("Starting existing container"));

    let exec = bondar(&[
        "exec",
        "--workspace-folder",
        ws_str,
        "--",
        "cat",
        "/tmp/ps.txt",
    ]);
    assert!(
        exec.status.success(),
        "postStart did not run: {}",
        String::from_utf8_lossy(&exec.stderr)
    );
    assert!(String::from_utf8_lossy(&exec.stdout).contains("started"));

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(down.status.success());
    cleanup(&ws);
}

#[test]
fn test_compose_one_off_container_is_ignored() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = std::env::temp_dir().join("bondar-int-compose-oneoff");
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join(".devcontainer")).unwrap();
    std::fs::write(
        ws.join("docker-compose.yml"),
        "services:\n  app:\n    image: ubuntu:22.04\n    command: sh -c 'while sleep 1000; do :; done'\n    volumes:\n      - .:/workspace\n",
    )
    .unwrap();
    std::fs::write(
        ws.join(".devcontainer/devcontainer.json"),
        r#"{"name": "int-compose-oneoff", "dockerComposeFile": "../docker-compose.yml", "service": "app", "workspaceFolder": "/workspace", "onCreateCommand": "sh -c 'echo run >> /tmp/oc-oneoff.txt'", "userEnvProbe": "none"}"#,
    )
    .unwrap();
    let ws_str = ws.to_str().unwrap();

    let up1 = bondar(&[
        "up",
        "--workspace-folder",
        ws_str,
        "--remove-existing-container",
    ]);
    assert!(
        up1.status.success(),
        "first up failed: {}",
        String::from_utf8_lossy(&up1.stderr)
    );
    assert!(String::from_utf8_lossy(&up1.stdout).contains("Running onCreateCommand"));

    let project = project_name_for(&ws);
    let compose_file = ws.join("docker-compose.yml");
    let compose_file = compose_file.to_str().unwrap();

    // Create a one-off container for the same service
    let run = Command::new("docker")
        .args([
            "compose",
            "--project-name",
            &project,
            "-f",
            compose_file,
            "run",
            "-d",
            "app",
            "sh",
            "-c",
            "sleep 600",
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "one-off run failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    // Remove the actual service container, leaving only the one-off
    let ps = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("label=com.docker.compose.project={project}"),
            "--filter",
            "label=com.docker.compose.service=app",
            "--format",
            "{{.ID}}\t{{.Label \"com.docker.compose.oneoff\"}}",
        ])
        .output()
        .unwrap();
    let service_id = String::from_utf8_lossy(&ps.stdout)
        .lines()
        .find_map(|line| {
            let mut parts = line.split('\t');
            let id = parts.next().unwrap_or("").trim();
            let oneoff = parts.next().unwrap_or("").trim();
            if !id.is_empty() && oneoff != "True" {
                Some(id.to_string())
            } else {
                None
            }
        })
        .expect("service container not found");
    assert!(
        Command::new("docker")
            .args(["rm", "-f", &service_id])
            .status()
            .unwrap()
            .success()
    );

    // The one-off must not be mistaken for the service container: `up` has to
    // treat the service as missing and run the create lifecycle again.
    let up2 = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up2.status.success(),
        "second up failed: {}",
        String::from_utf8_lossy(&up2.stderr)
    );
    assert!(
        String::from_utf8_lossy(&up2.stdout).contains("Running onCreateCommand"),
        "service container was not recreated: {}",
        String::from_utf8_lossy(&up2.stdout)
    );

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(down.status.success());

    // Remove any leftover project container (e.g. the one-off)
    let leftovers = Command::new("docker")
        .args([
            "ps",
            "-a",
            "-q",
            "--filter",
            &format!("label=com.docker.compose.project={project}"),
        ])
        .output()
        .unwrap();
    for id in String::from_utf8_lossy(&leftovers.stdout).lines() {
        let _ = Command::new("docker").args(["rm", "-f", id]).output();
    }
    cleanup(&ws);
}

#[test]
fn test_down_default_has_no_unknown_action_warning() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "down-default",
        r#"{"name": "int-down-default", "image": "ubuntu:22.04", "workspaceFolder": "/workspace"}"#,
    );
    let ws_str = ws.to_str().unwrap();

    // `down` on a non-existent container uses the internal "remove" default
    // and must not warn about an unknown shutdownAction.
    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(
        down.status.success(),
        "down failed: {}",
        String::from_utf8_lossy(&down.stderr)
    );
    let stderr = String::from_utf8_lossy(&down.stderr);
    assert!(
        !stderr.contains("unknown shutdownAction"),
        "spurious warning: {stderr}"
    );

    cleanup(&ws);
}

#[test]
fn test_image_metadata_remote_user() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "image-meta",
        r#"{"name": "int-image-meta", "image": "bondar-int-image-meta:1", "workspaceFolder": "/workspace", "userEnvProbe": "none"}"#,
    );
    std::fs::write(
        ws.join("Dockerfile"),
        "FROM ubuntu:22.04\nLABEL devcontainer.metadata='[{\"remoteUser\":\"vscode\",\"containerEnv\":{\"META_ENV\":\"1\"},\"privileged\":true,\"postCreateCommand\":\"echo meta-hook > /tmp/meta-hook.txt\"}]'\n",
    )
    .unwrap();
    let build = Command::new("docker")
        .args(["build", "-t", "bondar-int-image-meta:1"])
        .arg(&ws)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "image build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let ws_str = ws.to_str().unwrap();
    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up.status.success(),
        "up failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );

    // remoteUser comes from the image metadata label
    let exec = bondar(&["exec", "--workspace-folder", ws_str, "--", "id", "-un"]);
    assert!(
        exec.status.success(),
        "exec failed: {}",
        String::from_utf8_lossy(&exec.stderr)
    );
    assert!(
        String::from_utf8_lossy(&exec.stdout).contains("vscode"),
        "remoteUser from image metadata not applied: {}",
        String::from_utf8_lossy(&exec.stdout)
    );

    // containerEnv and privileged from the metadata label are applied as well
    let env = bondar(&[
        "exec",
        "--workspace-folder",
        ws_str,
        "--",
        "sh",
        "-c",
        "echo META_ENV=$META_ENV",
    ]);
    assert!(
        env.status.success() && String::from_utf8_lossy(&env.stdout).contains("META_ENV=1"),
        "containerEnv from image metadata not applied: {}",
        String::from_utf8_lossy(&env.stdout)
    );
    // Lifecycle hooks from the metadata label run on creation
    let hook = bondar(&[
        "exec",
        "--workspace-folder",
        ws_str,
        "--",
        "cat",
        "/tmp/meta-hook.txt",
    ]);
    assert!(
        hook.status.success() && String::from_utf8_lossy(&hook.stdout).contains("meta-hook"),
        "postCreateCommand from image metadata not executed: {}",
        String::from_utf8_lossy(&hook.stderr)
    );

    let privileged = Command::new("docker")
        .args([
            "inspect",
            "-f",
            "{{.HostConfig.Privileged}}",
            "bondar-int-image-meta",
        ])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&privileged.stdout).trim(),
        "true",
        "privileged from image metadata not applied"
    );

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(down.status.success());
    let _ = Command::new("docker")
        .args(["rmi", "-f", "bondar-int-image-meta:1"])
        .output();
    cleanup(&ws);
}

#[test]
fn test_compose_stop_container_only_primary() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = std::env::temp_dir().join("bondar-int-compose-stop-primary");
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join(".devcontainer")).unwrap();
    std::fs::write(
        ws.join("docker-compose.yml"),
        "services:\n  app:\n    image: ubuntu:22.04\n    command: sh -c 'while sleep 1000; do :; done'\n  db:\n    image: ubuntu:22.04\n    command: sh -c 'while sleep 1000; do :; done'\n",
    )
    .unwrap();
    std::fs::write(
        ws.join(".devcontainer/devcontainer.json"),
        r#"{"name": "int-compose-stop-primary", "dockerComposeFile": "../docker-compose.yml", "service": "app", "runServices": ["db"], "workspaceFolder": "/workspace", "shutdownAction": "stopContainer", "userEnvProbe": "none"}"#,
    )
    .unwrap();
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up.status.success(),
        "compose up failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(
        down.status.success(),
        "compose down failed: {}",
        String::from_utf8_lossy(&down.stderr)
    );

    // The primary service is stopped; the other service keeps running
    let project = project_name_for(&ws);
    let running = |service: &str| {
        let output = Command::new("docker")
            .args([
                "ps",
                "-q",
                "--filter",
                &format!("label=com.docker.compose.project={project}"),
                "--filter",
                &format!("label=com.docker.compose.service={service}"),
            ])
            .output()
            .unwrap();
        !output.stdout.is_empty()
    };
    // Poll briefly so slow CI does not flake
    let mut primary_stopped = false;
    let mut other_running = false;
    for _ in 0..30 {
        if !running("app") {
            primary_stopped = true;
        }
        if running("db") {
            other_running = true;
        }
        if primary_stopped && other_running {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(primary_stopped, "primary service should be stopped");
    assert!(other_running, "other service should keep running");

    // Cleanup the whole project
    let compose_file = ws.join("docker-compose.yml");
    let _ = Command::new("docker")
        .args([
            "compose",
            "--project-name",
            &project,
            "-f",
            compose_file.to_str().unwrap(),
            "down",
        ])
        .output();
    cleanup(&ws);
}

#[test]
fn test_stale_container_config_warning() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let content = r#"{"name": "int-stale", "image": "ubuntu:22.04", "workspaceFolder": "/workspace", "userEnvProbe": "none"}"#;
    let ws = make_workspace("stale", content);
    let ws_str = ws.to_str().unwrap();

    let up1 = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(up1.status.success());

    // Starting the same container through a different config file warns
    let alt = ws.join("alt.json");
    std::fs::write(&alt, content).unwrap();
    let alt_str = alt.to_str().unwrap();
    let up2 = bondar(&["up", "--workspace-folder", ws_str, "--config", alt_str]);
    assert!(
        up2.status.success(),
        "second up failed: {}",
        String::from_utf8_lossy(&up2.stderr)
    );
    assert!(
        String::from_utf8_lossy(&up2.stderr).contains("different config file"),
        "expected stale config warning: {}",
        String::from_utf8_lossy(&up2.stderr)
    );

    let down = bondar(&["down", "--workspace-folder", ws_str, "--config", alt_str]);
    assert!(down.status.success());
    cleanup(&ws);
}

#[test]
fn test_compose_down_removes_orphans() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = std::env::temp_dir().join("bondar-int-compose-orphans");
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join(".devcontainer")).unwrap();
    std::fs::write(
        ws.join("docker-compose.yml"),
        "services:\n  app:\n    image: ubuntu:22.04\n    command: sh -c 'while sleep 1000; do :; done'\n  db:\n    image: ubuntu:22.04\n    command: sh -c 'while sleep 1000; do :; done'\n",
    )
    .unwrap();
    std::fs::write(
        ws.join(".devcontainer/devcontainer.json"),
        r#"{"name": "int-compose-orphans", "dockerComposeFile": "../docker-compose.yml", "service": "app", "runServices": ["db"], "workspaceFolder": "/workspace", "userEnvProbe": "none"}"#,
    )
    .unwrap();
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up.status.success(),
        "compose up failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );

    // Remove the "db" service from the compose file: its container becomes an orphan
    std::fs::write(
        ws.join("docker-compose.yml"),
        "services:\n  app:\n    image: ubuntu:22.04\n    command: sh -c 'while sleep 1000; do :; done'\n",
    )
    .unwrap();

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(
        down.status.success(),
        "compose down failed: {}",
        String::from_utf8_lossy(&down.stderr)
    );

    let project = project_name_for(&ws);
    let leftover = Command::new("docker")
        .args([
            "ps",
            "-a",
            "-q",
            "--filter",
            &format!("label=com.docker.compose.project={project}"),
        ])
        .output()
        .unwrap();
    assert!(
        leftover.stdout.is_empty(),
        "orphan containers were not removed"
    );
    cleanup(&ws);
}

#[test]
fn test_compose_build_metadata_container_properties() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = std::env::temp_dir().join("bondar-int-compose-meta");
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join(".devcontainer")).unwrap();
    std::fs::write(
        ws.join("Dockerfile"),
        "FROM ubuntu:22.04\nLABEL devcontainer.metadata='[{\"containerEnv\":{\"META_ENV\":\"from-meta\"},\"privileged\":true}]'\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("docker-compose.yml"),
        "services:\n  app:\n    build:\n      context: .\n      dockerfile: Dockerfile\n    command: sh -c 'while sleep 1000; do :; done'\n",
    )
    .unwrap();
    std::fs::write(
        ws.join(".devcontainer/devcontainer.json"),
        r#"{"name": "int-compose-meta", "dockerComposeFile": "../docker-compose.yml", "service": "app", "workspaceFolder": "/workspace", "userEnvProbe": "none"}"#,
    )
    .unwrap();
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up.status.success(),
        "compose up failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );

    let id = Command::new("docker")
        .args([
            "ps",
            "-a",
            "-q",
            "--filter",
            &format!("label=com.docker.compose.project={}", project_name_for(&ws)),
        ])
        .output()
        .unwrap();
    let id = String::from_utf8_lossy(&id.stdout).trim().to_string();
    assert!(!id.is_empty(), "service container not found");

    let privileged = Command::new("docker")
        .args(["inspect", "-f", "{{.HostConfig.Privileged}}", &id])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&privileged.stdout).trim(),
        "true",
        "metadata privileged not applied to the compose container"
    );

    let env = Command::new("docker")
        .args(["exec", &id, "sh", "-c", "echo META_ENV=$META_ENV"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&env.stdout).contains("META_ENV=from-meta"),
        "metadata containerEnv not applied to the compose container"
    );

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(down.status.success());
    let _ = Command::new("docker")
        .args(["rmi", "-f", &format!("{}-app", project_name_for(&ws))])
        .output();
    cleanup(&ws);
}

#[test]
fn test_compose_stop_compose_without_primary_container() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = std::env::temp_dir().join("bondar-int-compose-stop-noprimary");
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join(".devcontainer")).unwrap();
    std::fs::write(
        ws.join("docker-compose.yml"),
        "services:\n  app:\n    image: ubuntu:22.04\n    command: sh -c 'while sleep 1000; do :; done'\n  db:\n    image: ubuntu:22.04\n    command: sh -c 'while sleep 1000; do :; done'\n",
    )
    .unwrap();
    std::fs::write(
        ws.join(".devcontainer/devcontainer.json"),
        r#"{"name": "int-compose-stop-noprimary", "dockerComposeFile": "../docker-compose.yml", "service": "app", "runServices": ["db"], "workspaceFolder": "/workspace", "shutdownAction": "stopCompose", "userEnvProbe": "none"}"#,
    )
    .unwrap();
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up.status.success(),
        "compose up failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );

    // Remove the primary service container manually, leaving only "db"
    let project = project_name_for(&ws);
    let app = Command::new("docker")
        .args([
            "ps",
            "-a",
            "-q",
            "--filter",
            &format!("label=com.docker.compose.project={project}"),
            "--filter",
            "label=com.docker.compose.service=app",
        ])
        .output()
        .unwrap();
    let app_id = String::from_utf8_lossy(&app.stdout).trim().to_string();
    assert!(!app_id.is_empty());
    assert!(
        Command::new("docker")
            .args(["rm", "-f", &app_id])
            .status()
            .unwrap()
            .success()
    );

    // stopCompose must still stop the remaining services
    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(
        down.status.success(),
        "compose down failed: {}",
        String::from_utf8_lossy(&down.stderr)
    );
    let db = Command::new("docker")
        .args([
            "ps",
            "-q",
            "--filter",
            &format!("label=com.docker.compose.project={project}"),
            "--filter",
            "label=com.docker.compose.service=db",
        ])
        .output()
        .unwrap();
    assert!(
        db.stdout.is_empty(),
        "remaining compose service was not stopped"
    );

    // Cleanup the stopped project
    let compose_file = ws.join("docker-compose.yml");
    let _ = Command::new("docker")
        .args([
            "compose",
            "--project-name",
            &project,
            "-f",
            compose_file.to_str().unwrap(),
            "down",
        ])
        .output();
    cleanup(&ws);
}

#[test]
fn test_container_exiting_immediately_warns() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "exits",
        r#"{"name": "int-exits", "image": "bondar-int-exits:1", "workspaceFolder": "/workspace", "overrideCommand": false, "userEnvProbe": "none"}"#,
    );
    // An entrypoint that exits immediately makes the check deterministic
    std::fs::write(
        ws.join("Dockerfile"),
        "FROM ubuntu:22.04\nENTRYPOINT [\"/bin/false\"]\n",
    )
    .unwrap();
    let build = Command::new("docker")
        .args(["build", "-t", "bondar-int-exits:1"])
        .arg(&ws)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "image build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&[
        "up",
        "--workspace-folder",
        ws_str,
        "--remove-existing-container",
    ]);
    assert!(
        up.status.success(),
        "up failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );
    assert!(
        String::from_utf8_lossy(&up.stderr).contains("is not running"),
        "expected an exited-container warning: {}",
        String::from_utf8_lossy(&up.stderr)
    );

    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(down.status.success());
    let _ = Command::new("docker")
        .args(["rmi", "-f", "bondar-int-exits:1"])
        .output();
    cleanup(&ws);
}

#[test]
fn test_shutdown_action_from_image_metadata() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "shutdown-meta",
        r#"{"name": "int-shutdown-meta", "image": "bondar-int-shutdown-meta:1", "workspaceFolder": "/workspace", "userEnvProbe": "none"}"#,
    );
    std::fs::write(
        ws.join("Dockerfile"),
        "FROM ubuntu:22.04\nLABEL devcontainer.metadata='[{\"shutdownAction\":\"stopContainer\"}]'\n",
    )
    .unwrap();
    let build = Command::new("docker")
        .args(["build", "-t", "bondar-int-shutdown-meta:1"])
        .arg(&ws)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "image build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(
        up.status.success(),
        "up failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );

    // Without a config shutdownAction, the image metadata says stopContainer
    let down = bondar(&["down", "--workspace-folder", ws_str]);
    assert!(
        down.status.success(),
        "down failed: {}",
        String::from_utf8_lossy(&down.stderr)
    );
    let running = Command::new("docker")
        .args([
            "inspect",
            "-f",
            "{{.State.Running}}",
            "bondar-int-shutdown-meta",
        ])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&running.stdout).trim(),
        "false",
        "container should be stopped and kept"
    );

    let _ = Command::new("docker")
        .args(["rm", "-f", "bondar-int-shutdown-meta"])
        .output();
    let _ = Command::new("docker")
        .args(["rmi", "-f", "bondar-int-shutdown-meta:1"])
        .output();
    cleanup(&ws);
}

#[test]
fn test_name_and_container_name_warning() {
    let ws = make_workspace(
        "name-conflict",
        r#"{"name": "my-name", "containerName": "my-container", "image": "ubuntu:22.04"}"#,
    );
    let ws_str = ws.to_str().unwrap();
    // `build` goes through load_config (read-configuration does not)
    let out = bondar(&["build", "--workspace-folder", ws_str]);
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("both 'name' and 'containerName'"),
        "expected a precedence warning: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    cleanup(&ws);
}

#[test]
fn test_workspace_mount_folder_mismatch_warning() {
    let ws = make_workspace(
        "mount-mismatch",
        r#"{"name": "int-mount-mismatch", "image": "ubuntu:22.04", "workspaceFolder": "/workspace", "workspaceMount": "type=bind,source=/tmp,target=/other"}"#,
    );
    let ws_str = ws.to_str().unwrap();
    // `build` goes through load_config (validation emits the warning)
    let out = bondar(&["build", "--workspace-folder", ws_str]);
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("differs from workspaceFolder"),
        "expected a mismatch warning: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    cleanup(&ws);
}

#[test]
fn test_stopcompose_without_compose_warns() {
    let ws = make_workspace(
        "stopcompose-image",
        r#"{"name": "int-stopcompose-image", "image": "ubuntu:22.04", "shutdownAction": "stopCompose"}"#,
    );
    let ws_str = ws.to_str().unwrap();
    let out = bondar(&["build", "--workspace-folder", ws_str]);
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("requires dockerComposeFile"),
        "expected a stopCompose warning: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    cleanup(&ws);
}

#[test]
fn test_build_no_cache_without_build_warns() {
    if !docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }
    let ws = make_workspace(
        "nocache-image",
        r#"{"name": "int-nocache-image", "image": "ubuntu:22.04"}"#,
    );
    let ws_str = ws.to_str().unwrap();
    let out = bondar(&["build", "--workspace-folder", ws_str, "--no-cache"]);
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--no-cache has no effect"),
        "expected a --no-cache warning: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    cleanup(&ws);
}
