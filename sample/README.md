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
bondar up --workspace-folder ./sample
```
