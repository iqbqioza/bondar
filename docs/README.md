# bondar documentation

Documentation for **bondar**, a standalone devcontainer alternative that does not depend on Node.js or VSCode Server.

## Table of contents

- [User manual](manual.md) - installation, commands, configuration reference, lifecycle, environment variables, features, compose
- [FAQ](faq.md) - frequently asked questions
- [Troubleshooting](troubleshooting.md) - common issues and their solutions
- [Images](images/bondar-small.png) - project logo (CC0, 1254×1254 PNG)

## Quick reference

| Command | Description |
|---|---|
| `bondar build` | Build the dev container image |
| `bondar up` | Create and start the dev container |
| `bondar down` | Stop and remove the dev container |
| `bondar exec -- <cmd>` | Run a command inside the container |
| `bondar shell` | Start an interactive shell |
| `bondar logs` | Show container logs |
| `bondar read-configuration` | Validate and inspect the configuration |

Global flags: `--workspace-folder <DIR>` (defaults to the current directory), `--config <PATH>` (overrides the devcontainer.json path). Both may appear before or after the subcommand.

## Feature coverage

| Feature | Status |
|---|---|
| `image` / `build` / `dockerComposeFile` | Supported |
| Lifecycle scripts (6 types) | Supported (String/Array/Object, background via `waitFor`) |
| `containerEnv` / `remoteEnv` / `secrets` | Supported |
| `features` / `overrideFeatureInstallOrder` | Supported (OCI fetch + `install.sh`) |
| `mounts` / `workspaceMount` / `forwardPorts` / `appPort` | Supported (incl. ranges, IPv6, UDP) |
| `hostRequirements` / `updateRemoteUserUID` | Supported |
| `userEnvProbe` | Supported |
| `portsAttributes` / `otherPortsAttributes` | Supported |
| `shutdownAction` / `waitFor` | Supported |
| `read-configuration` | Strict JSON Schema validation |
| `customizations` | Intentionally unsupported (VSCode independence) |

## Platform support

bondar runs on **Linux**, **macOS** and **Windows**:

- Host requirements probes use platform-appropriate tools (`/proc/meminfo` on Linux, `sysctl` on macOS).
- Host lifecycle commands use `sh -c` on Unix and `cmd /C` on Windows.
- UID/GID synchronization is skipped on Windows (no POSIX UID concept).
- Temporary files (feature cache, compose override) use the OS temp directory.

## Project layout

```
docs/                    this documentation
sample/                  verification workspace
src/
  cli.rs                 command line definitions
  config.rs              devcontainer.json parser + validation
  docker.rs              Docker CLI wrapper
  compose.rs             Docker Compose support + override generation
  lifecycle.rs           lifecycle execution (sync/async) + child reaping
  features.rs            OCI feature fetch / transfer / install
  host.rs                host requirements + UID/GID sync
  command/               subcommand implementations
tests/
  docker_integration.rs  docker-backed integration tests
```