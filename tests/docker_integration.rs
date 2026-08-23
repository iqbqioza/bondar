use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_bondar");

/// Same FNV-1a hash bondar uses for the per-workspace compose project name.
fn project_name_for(ws: &std::path::Path) -> String {
    let ws_str = ws.to_string_lossy().to_string();
    let mut hash: u64 = 14695981039346656037;
    for b in ws_str.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("bondar-{}", &format!("{hash:016x}")[..8])
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
        .args(["rmi", "-f", "bondar-int-build"])
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
        r#"{"name": "int-compose", "dockerComposeFile": "../docker-compose.yml", "service": "app", "workspaceFolder": "/workspace"}"#,
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
        r#"{"name": "int-execopts", "image": "ubuntu:22.04", "workspaceFolder": "/workspace", "userEnvProbe": "none"}"#,
    );
    let ws_str = ws.to_str().unwrap();

    let up = bondar(&["up", "--workspace-folder", ws_str]);
    assert!(up.status.success());

    let exec = bondar(&[
        "exec",
        "--workspace-folder",
        ws_str,
        "--user",
        "root",
        "--workdir",
        "/tmp",
        "--",
        "pwd",
    ]);
    assert!(
        exec.status.success(),
        "exec failed: {}",
        String::from_utf8_lossy(&exec.stderr)
    );
    assert!(String::from_utf8_lossy(&exec.stdout).contains("/tmp"));

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
        .args(["rmi", "-f", "bondar-int-nocache"])
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
