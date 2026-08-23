# bondar FAQ

## General

### What is bondar?

bondar is a standalone alternative to the `devcontainer` CLI. It reads the same `.devcontainer/devcontainer.json` definitions and manages development containers, but it does not require Node.js or VSCode Server.

### Do I need Node.js?

No. bondar is a single Rust binary and only talks to the Docker CLI.

### Which platforms are supported?

Linux, macOS and Windows. Host-specific behavior is adapted (memory probes, shell selection, UID/GID handling).

### Do I need Docker Desktop?

Yes, you need a Docker daemon reachable via the `docker` CLI.

## Configuration

### Why does `bondar up` fail with "Docker CLI not found in PATH"?

The `docker` executable is not on `PATH`. Install Docker and make sure `docker` is callable, then retry.

### What does "Docker daemon not reachable" mean?

The `docker` CLI exists but the daemon (Docker Desktop or dockerd) is not running. Start Docker and retry.

### Can I use `image` and `build` together?

No. Validation rejects configurations that specify more than one of `image`, `build` or `dockerComposeFile`.

### Why is `workspaceFolder` required with `workspaceMount`?

The spec requires `workspaceFolder` so the tool knows where the mounted workspace is located inside the container.

### Why are empty strings rejected for `name`, `image`, etc.?

Empty values produce invalid Docker commands (e.g. `--name bondar-`, `-e =value`). Rejecting them early makes errors clearer.

## Environment and secrets

### How do I pass a host environment variable into the container?

Use `containerEnv` with `${localEnv:VAR}`:

```json
{
  "containerEnv": {"TOKEN": "${localEnv:MY_TOKEN}"}
}
```

### Why are `secrets` with a file path skipped?

The devcontainer spec supports two `secrets` forms: `{"localEnv": "VAR"}` and a file path. bondar implements the `localEnv` form; the file-path form is warned and skipped to avoid silently breaking builds.

### What does `${containerEnv:VAR}` resolve to?

It is resolved from the host environment before the container starts (the container does not exist yet at expansion time). Prefer `${localEnv:VAR}` unless you intentionally want the same semantics.

## Features

### How are features installed?

1. `oras pull <feature-id>` (or `docker pull` as a fallback).
2. Archives are extracted if necessary.
3. Files are copied into the container with `docker cp`.
4. `install.sh` is executed as root with the options passed as environment variables.

### Why is my feature not installed?

- `oras` is not installed and `docker pull` failed (OCI artifacts cannot be pulled as images).
- `install.sh` is missing from the artifact.
- The container already exists; features are only installed on creation (use `--remove-existing-container`).

### Can I control the install order?

Yes: `overrideFeatureInstallOrder`, or the `installsAfter` option on each feature.

## Compose

### Why is my `containerEnv` not in the compose service?

It is injected through a generated `compose.override.yml` stored in the OS temp directory. If you inspect `docker compose config`, use the same `-f` files bondar uses (the base file plus the override).

### Why was my `tmpfs` mount skipped?

`tmpfs` mounts have no short-syntax equivalent in compose. Declare them in the compose file instead.

### Why did `bondar down` run `docker compose stop`?

Because `shutdownAction` is `"stopCompose"` in your configuration. Unset it (or use `"none"`) for other behaviors.

## Ports

### How do I publish an IPv6 address?

Use bracket form: `"forwardPorts": ["[::1]:8080"]` or `"appPort": ["[::1]:8443"]`.

### How do I publish a range of ports?

`"forwardPorts": ["8080-8085"]` publishes `8080-8085` on both host and container.

### Why is my port not published?

Check `portsAttributes`/`otherPortsAttributes` for `"onAutoForward": "ignore"` - such ports are skipped by design.

## Users and permissions

### Why is the `vscode` user created automatically?

`updateRemoteUserUID` (default `true`) creates the `remoteUser` if it does not exist in the image, so that bind-mounted files keep the host UID/GID.

### Why is UID/GID synchronization skipped?

On Windows (no POSIX UID concept), or when bondar runs as root on Unix (mapping to uid 0 is pointless).

### Can I disable it?

Yes: set `"updateRemoteUserUID": false`.

## Misc

### Where does bondar store temporary files?

In the OS temp directory (`/tmp` on Linux, `%TEMP%` on Windows): the feature cache (`bondar_features/`) and the compose override file.

### Does bondar leave zombie processes?

No. Background lifecycle scripts are tracked and reaped at exit.

### Why does `read-configuration` exit with code 1?

The configuration failed JSON Schema validation or a cross-field check (e.g. `image` + `build` both set).

### Can I use bondar with a repository that has `customizations`?

Yes. `customizations` is parsed but intentionally ignored (no VSCode dependency), so the container still works.