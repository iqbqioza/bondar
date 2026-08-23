# bondar sample

Minimal devcontainer workspace for testing `bondar`.

## Image mode (default)

`sample/.devcontainer/devcontainer.json` uses `image` directly:

```json
{
  "name": "bondar-sample",
  "image": "mcr.microsoft.com/devcontainers/base:ubuntu"
}
```

Run:

```sh
bondar up --workspace-folder ./sample
bondar exec --workspace-folder ./sample -- ls -la
bondar shell --workspace-folder ./sample
bondar down --workspace-folder ./sample
```

## Build mode

To test `build` instead of `image`, replace `devcontainer.json` with:

```json
{
  "name": "bondar-sample-build",
  "build": {
    "dockerfile": "Dockerfile",
    "context": "."
  },
  "workspaceFolder": "/workspace"
}
```

Then:

```sh
bondar build --workspace-folder ./sample
bondar build --workspace-folder ./sample --no-cache
bondar up --workspace-folder ./sample
```

## Lifecycle (implemented)

```json
{
  "initializeCommand": "echo init on host",
  "onCreateCommand": "echo onCreate inside container",
  "updateContentCommand": ["echo", "update"],
  "postCreateCommand": {"step1": "echo hello", "step2": ["echo", "world"]},
  "postStartCommand": "echo start",
  "postAttachCommand": "echo attach"
}
```

- `initializeCommand` runs on the host (`workspaceFolder` as cwd)
- `onCreate`/`updateContent`/`postCreate` run only on the first `up` (new container)
- `postStart` runs when the container was stopped then started
- `postAttach` runs on every `up`

String => `sh -c`, Array => direct exec, Object => sequential per key.

## Env and variables

```json
{
  "containerEnv": {"FOO": "${localEnv:HOME}"},
  "remoteEnv": {"BAR": "${containerWorkspaceFolder}"},
  "runArgs": ["--label", "my=${localEnv:VAR}"]
}
```

Supported expansions: `${localWorkspaceFolder}`, `${localWorkspaceFolderBasename}`, `${containerWorkspaceFolder}`, `${containerWorkspaceFolderBasename}`, `${localEnv:VAR:default}`, `${containerEnv:VAR:default}`, `${devcontainerId}`.
Labels `devcontainer.local_folder`, `devcontainer.config_file`, `devcontainer.id` are auto-added to `docker run`.

## Feature support

- `features` / `overrideFeatureInstallOrder` -> supported (requires `oras` or `docker pull` to fetch, then executes `install.sh` inside the container)
- `dockerComposeFile` / `service` / `runServices` -> supported (`docker compose` with dynamic override.yml injection of env/mounts/ports)
- `hostRequirements` / `updateRemoteUserUID` / `shutdownAction` -> supported
- `portsAttributes` / `otherPortsAttributes` -> stored as container labels (no UI)