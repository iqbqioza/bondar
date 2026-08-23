# bondar user manual

## Installation

Requirements:

- A Rust toolchain (only for building)
- The Docker CLI (`docker`) available on `PATH`
- Docker Desktop (macOS/Windows) or the Docker daemon (Linux)

```sh
cargo build --release
# add target/release/bondar to your PATH
```

## Global options

```sh
bondar [--workspace-folder <DIR>] [--config <PATH>] <COMMAND>
```

- `--workspace-folder <DIR>`: the workspace containing `.devcontainer/devcontainer.json` or `.devcontainer.json`. Defaults to the current directory. Must be a directory.
- `--config <PATH>`: overrides the devcontainer.json path.

Both flags are global and may be placed before or after the subcommand.

## Commands

### `bondar build`

Builds the image defined by `build` in `devcontainer.json`.

```sh
bondar build [--workspace-folder <DIR>] [--no-cache]
```

- `--no-cache`: passes `--no-cache` to `docker build`.
- With `dockerComposeFile`, runs `docker compose build` (with `--no-cache` when requested).
- Build args, `target`, `cacheFrom` and `options` are passed through; variables (`${localEnv:...}` etc.) are expanded.

### `bondar up`

Creates and starts the dev container.

```sh
bondar up [--workspace-folder <DIR>] [--remove-existing-container] [--no-build] [--no-cache]
```

- `--remove-existing-container`: removes an existing container before recreating it (runs the create lifecycle again).
- `--no-build`: skips building even when a `build` section is configured.
- `--no-cache`: builds with `--no-cache` (compose: `docker compose build --no-cache`).

The `up` flow:

1. Validates the configuration.
2. Runs `initializeCommand` on the host.
3. Builds the image (unless `--no-build`).
4. Creates/starts the container with all options (mounts, env, ports, labels).
5. Synchronizes the container user UID/GID with the host (`updateRemoteUserUID`).
6. Installs features (first creation only).
7. Probes the user environment (`userEnvProbe`).
8. Runs lifecycle scripts in order (`onCreateCommand`, `updateContentCommand`, `postCreateCommand`, `postStartCommand`, `postAttachCommand`).

With `waitFor`, scripts after the specified one run in the background.

### `bondar down`

Stops and removes the dev container.

```sh
bondar down [--workspace-folder <DIR>]
```

Behavior depends on `shutdownAction`:

- unset: remove the container (`docker rm -f`)
- `"stopContainer"`: stop only, keep the container
- `"none"`: do nothing
- compose + `"stopCompose"`: `docker compose stop`
- compose (unset or other): `docker compose down`

### `bondar exec -- <command>...`

Runs a command inside the running container.

```sh
bondar exec [--user <USER>] [--workdir <DIR>] -- <command>...
```

- `--user`: overrides `remoteUser`/`containerUser`.
- `--workdir`: overrides `workspaceFolder` as the working directory.
- `remoteEnv` and probed environment variables are applied.
- The exit code of the command is propagated.
- Interactive TTY is enabled when stdin and stdout are terminals.

### `bondar shell`

Starts an interactive shell (bash if present, otherwise sh) inside the container.

### `bondar logs`

Shows container logs.

```sh
bondar logs [--follow] [--tail <LINES>]
```

- `--follow`: follows the log output (`docker logs -f` / `docker compose logs -f`).
- `--tail <LINES>`: shows only the last N lines (numeric values only).

### `bondar read-configuration`

Validates the configuration against the official devcontainer JSON Schema and prints it.

```sh
bondar read-configuration [--include-merged-configuration]
```

- Prints the resolved configuration (typed fields plus unknown/custom fields).
- Validates against `devContainer.base.schema.json` (bundled).
- Exits with code 1 when the configuration is invalid.
- `--include-merged-configuration`: also prints a merged view (expanded env, secrets, ports, container name, defaults).

## Configuration reference

### Image / Dockerfile

```json
{
  "name": "my-dev",
  "image": "mcr.microsoft.com/devcontainers/base:ubuntu",
  "workspaceFolder": "/workspace"
}
```

```json
{
  "name": "my-dev",
  "build": {
    "dockerfile": "Dockerfile",
    "context": "..",
    "args": {"VARIANT": "3.1"},
    "target": "development",
    "cacheFrom": "my-registry/my-image:latest",
    "options": ["--add-host=host.docker.internal:host-gateway"]
  }
}
```

Constraints enforced by validation:

- Exactly one of `image`, `build` or `dockerComposeFile`.
- `dockerComposeFile` requires `service`.
- `workspaceMount` requires `workspaceFolder`.
- Empty strings are rejected for `name`, `image`, `service`, `workspaceFolder`, `build.dockerfile`, `dockerComposeFile`.
- `containerEnv`/`remoteEnv` keys must not be empty.

### Environment variables

```json
{
  "containerEnv": {"PATH_EXTRA": "/opt/bin", "FROM_HOST": "${localEnv:HOME}"},
  "remoteEnv": {"WS": "${containerWorkspaceFolder}"},
  "secrets": {"MY_SECRET": {"localEnv": "SECRET_VALUE"}}
}
```

Supported variable expansion:

| Variable | Meaning |
|---|---|
| `${localWorkspaceFolder}` | Absolute host path of the workspace |
| `${localWorkspaceFolderBasename}` | Base name of the workspace folder |
| `${containerWorkspaceFolder}` | Container-side workspace path (`workspaceFolder`) |
| `${containerWorkspaceFolderBasename}` | Base name of the container workspace path |
| `${localEnv:VAR[:default]}` | Value of the host environment variable |
| `${containerEnv:VAR[:default]}` | Value of the environment variable (resolved from `containerEnv` when set, otherwise from the host) |
| `${devcontainerId}` | Stable identifier derived from the workspace path |

`secrets` supports the `{"localEnv": "VAR"}` form. The file-path string form is warned and skipped. Both the JSON Schema validation in `read-configuration` and bondar itself accept the `localEnv` form.

### Mounts

```json
{
  "mounts": [
    "type=bind,source=${localWorkspaceFolder}/data,target=/data,readonly",
    {"type": "volume", "source": "myvol", "target": "/data", "readonly": true}
  ]
}
```

- String mounts use Docker `--mount` syntax.
- Object mounts support `source`, `target`, `type`, `readonly`.
- For compose, string mounts are converted to short syntax; `tmpfs` mounts have no short-syntax equivalent and are skipped with a warning.

### Ports

```json
{
  "forwardPorts": [3000, "db:5432", "127.0.0.1:9090", "8080-8085"],
  "appPort": ["8080:80", "[::1]:8443"]
}
```

- Plain numbers publish the same port on host and container.
- `host:container` and `ip:host:container` forms are supported.
- Port ranges (`8080-8085`) are supported.
- IPv6 addresses use bracket form (`[::1]:8080`).
- `/udp` suffix and `portsAttributes` `protocol: "udp"` are honored.
- `portsAttributes`/`otherPortsAttributes` `onAutoForward: "ignore"` disables publishing.
- `portsAttributes` keys are matched against the *container port number* (or range). Regex keys (e.g. `.+/server.js`) cannot be evaluated at publish time and are ignored, since bondar has no running process list.

### Lifecycle scripts

```json
{
  "initializeCommand": "echo init on host",
  "onCreateCommand": "npm install",
  "updateContentCommand": ["git", "pull"],
  "postCreateCommand": {"step1": "echo hello", "step2": ["echo", "world"]},
  "postStartCommand": "npm run dev",
  "postAttachCommand": "echo attached",
  "waitFor": "postCreateCommand"
}
```

- String values run through the shell (`sh -c` on Unix, `cmd /C` on Windows); the whole string is one shell command line.
- Array values run directly without a shell.
- Object values run each key sequentially (in declaration order).
- `initializeCommand` runs on the host; the others run inside the container.
- `waitFor` accepts `initializeCommand`, `onCreateCommand`, `updateContentCommand`, `postCreateCommand`, `postStartCommand`; scripts after the selected one run in the background.
- Script failures stop subsequent scripts.

Note: the devcontainer spec defaults `waitFor` to `updateContentCommand` (so later scripts run in the background while an editor starts). bondar runs all lifecycle scripts synchronously unless `waitFor` is set, since a terminal CLI has no UI to start.

### Features

```json
{
  "features": {
    "ghcr.io/devcontainers/features/common-utils:2": {"installZsh": true}
  },
  "overrideFeatureInstallOrder": ["ghcr.io/devcontainers/features/common-utils:2"]
}
```

- Features are fetched with `oras` (or `docker pull` as a fallback), extracted if needed, copied into the container, and executed via `install.sh` as root.
- Options are passed as environment variables to `install.sh` (`installsAfter` is excluded).
- `installsAfter` (from the feature metadata or options) orders independent features; unknown dependencies are warned.
- Features are installed only when the container is created, not on restart.
- `customizations` declared by features are merged and stored as a container label.

### Compose

```json
{
  "dockerComposeFile": ["../docker-compose.yml", "../docker-compose.override.yml"],
  "service": "app",
  "runServices": ["app", "db"],
  "workspaceFolder": "/workspace",
  "shutdownAction": "stopCompose"
}
```

- `dockerComposeFile` may be a string or an array; paths are relative to `devcontainer.json`, and `${localWorkspaceFolder}` expands to the workspace root.
- `runServices` limits the services started.
- `containerEnv`, `secrets`, `forwardPorts`, `appPort` and `mounts` are injected via a generated `compose.override.yml` (in the OS temp directory).
- `--remove-existing-container` passes `--force-recreate`; `--no-build` passes `--no-build`.
- Each workspace gets a stable per-workspace compose project name (`bondar-<hash>`), so workspaces with the same directory name never share containers, networks or override files.

### Host requirements

```json
{
  "hostRequirements": {
    "cpus": 4,
    "memory": "8gb",
    "storage": "32gb",
    "gpu": true
  }
}
```

Warnings are emitted when the host does not satisfy the requirements (not enforced as errors).

## Lifecycle of the container user

`updateRemoteUserUID` (default `true`) synchronizes the `remoteUser`/`containerUser` UID and GID with the host:

1. If the user does not exist in the container, it is created (`groupadd` + `useradd`).
2. `usermod`/`groupmod` update UID/GID (primary group is resolved first).
3. The workspace directory is chowned to the user.
4. On Windows, UID/GID synchronization is skipped.

## Output and exit codes

- Normal output goes to stdout; warnings go to stderr.
- `exec`/`shell` propagate the container command exit code.
- `read-configuration` exits with code 1 when the configuration is invalid.
- All other errors exit with code 1.

## Background processes

Scripts launched in the background (via `waitFor`) are tracked and reaped at exit to avoid zombie processes.