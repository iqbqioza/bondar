# bondar troubleshooting

## Docker

### "Docker CLI not found in PATH"

**Cause:** the `docker` executable is not on `PATH`.

**Fix:** install Docker (or Docker Desktop) and verify:

```sh
docker --version
```

Then retry the command.

### "Docker daemon not reachable"

**Cause:** the `docker` CLI exists but the daemon is not running.

**Fix:** start Docker Desktop (macOS/Windows) or `dockerd` (Linux), wait until `docker ps` works, then retry.

## Container startup

### Container created but exits immediately

**Possible causes:**

- The image has no long-running process (`init` or an entrypoint). Most devcontainer images ship with `init`; if yours does not, add `"init": true` to `devcontainer.json`.
- A lifecycle script crashed the container.

**Debug steps:**

1. `bondar logs` to inspect the container output.
2. `bondar up` with a minimal configuration (drop lifecycle scripts) to isolate the cause.
3. Inspect the container directly: `docker ps -a`.

### "Container not running" when using `exec` / `shell` / `logs`

**Cause:** the container is stopped or was removed.

**Fix:** start it again:

```sh
bondar up
```

### Container is recreated unexpectedly

**Cause:** `--remove-existing-container` was passed, or the container label data changed.

**Fix:** avoid `--remove-existing-container` unless you intend to recreate. Container identity is tracked with the `devcontainer.id` label derived from the workspace path.

## Builds

### "Failed to build image"

**Possible causes:**

- `build.context` does not exist or is not a directory.
- `build.dockerfile` does not exist relative to the context.
- Build args reference undefined variables.
- Network issues pulling base images.

**Fix:** run `docker build` manually with the same context to see the underlying error.

### Build uses stale cache unexpectedly

Pass `--no-cache`:

```sh
bondar build --no-cache
```

## Features

### Feature is not installed

**Checklist:**

1. `oras` is installed, or `docker pull` must work (OCI artifacts cannot be pulled as ordinary images).
2. The feature artifact contains `install.sh`.
3. The container was created *after* the feature was added - features are only installed on creation. Use `--remove-existing-container` to recreate.

### "Failed to fetch feature" with oras

**Cause:** `oras` is not installed, the registry is unreachable, or the feature ID is wrong.

**Fix:** install `oras`, or verify the feature ID:

```sh
oras pull ghcr.io/devcontainers/features/common-utils:2
```

### Installation succeeds but the tool is missing

**Possible causes:**

- `install.sh` targets a different user or shell profile.
- The tool installs into a path not in `PATH` for your user.

**Fix:** check the feature's metadata for install location, and inspect the container:

```sh
bondar exec -- bash -lc 'ls -la /usr/local/share/'
```

## Compose

### `docker compose` fails with "no such service"

**Cause:** `service` in `devcontainer.json` does not match a service defined in the compose file.

**Fix:** verify the service name:

```sh
docker compose -f docker-compose.yml config --services
```

### Environment variables from `containerEnv` are missing

**Possible causes:**

- The override file was not written (check that `containerEnv` is non-empty).
- The service name in the override does not match the base compose file.

**Fix:** inspect the effective configuration:

```sh
docker compose -f docker-compose.yml -f compose.override.yml config
```

The override file is written to the OS temp directory by `compose_base_command`.

### `bondar down` behaves differently than expected

**Cause:** `shutdownAction` controls the behavior:

| `shutdownAction` | `bondar down` |
|---|---|
| unset (image-based) | `docker rm -f` |
| `"stopContainer"` | `docker stop` (container kept) |
| `"none"` | no action |
| `"stopCompose"` (compose) | `docker compose stop` |
| unset (compose) | `docker compose down` |

## Environment variables

### `${localEnv:VAR}` expands to nothing

**Cause:** the variable is not set in the host environment.

**Fix:** set it, or use a default:

```json
{"containerEnv": {"MY_VAR": "${localEnv:MY_VAR:fallback}"}}
```

### `${containerEnv:VAR}` does not work as expected

**Cause:** for variables set in the same `containerEnv`, bondar resolves them from `containerEnv`. For anything else it falls back to the host environment, because the container does not exist yet at expansion time. If you need the *container's* runtime environment, use `remoteEnv` with a literal value or a script that runs in the container.

## Ports

### Port is not published

**Checklist:**

1. `portsAttributes` / `otherPortsAttributes` do not set `"onAutoForward": "ignore"` for that port.
2. The port is inside a range that matches the syntax `host:container` vs plain number (a plain number publishes the same port on both sides).
3. The container actually listens on the container-side port.

### IPv6 address fails

**Cause:** IPv6 must use bracket form, and the Docker daemon must have IPv6 enabled.

**Fix:** use `"[::1]:8080"` syntax; verify with `docker run --rm -p "[::1]:8080:8080" alpine echo ok`.

## Users and permissions

### Files created in the container have wrong ownership

**Cause:** `updateRemoteUserUID` could not map the UID/GID (e.g. custom user, Windows).

**Fix:** check `remoteUser`/`containerUser` is correct, or set `"updateRemoteUserUID": false` and manage ownership yourself.

### `useradd` fails during UID sync

**Cause:** the image's base distribution has an unusual useradd variant, or the UID/GID already exists.

**Fix:** prefer images that already define the target user, or disable sync with `"updateRemoteUserUID": false`.

## Misc

### `read-configuration` exits with code 1

Run with the workspace folder set and inspect the stderr output - it lists the exact validation failures.

### "Warning: secret key 'X' conflicts" appears

**Cause:** a `secrets` entry overrides a `containerEnv`/`remoteEnv` key with the same name.

**Fix:** rename one of them.

### Background lifecycle script output is lost

**Cause:** scripts after `waitFor` run in the background; bondar does not wait for them, so their output is interleaved with the foreground run.

**Fix:** set `waitFor` to a later script if you need the output, or check the container with `bondar logs`.

### Error messages are hard to read in CI

All diagnostics go to stderr; stdout only carries command output. Redirect stderr separately if needed:

```sh
bondar up 2>bondar.log
```