#!/bin/sh
#
# bondar installer
#
# Downloads the latest bondar release binary from GitHub and installs it into
# a user-writable directory that is already on PATH, so no sudo is required.
#
# Usage:
#   sh install.sh
#   curl -fsSL https://raw.githubusercontent.com/iqbqioza/bondar/main/install.sh | sh
#
# If a bondar binary already exists at the install target, you are asked to
# confirm the overwrite (answer 'y' to proceed).
#
# Internal/testing overrides (optional):
#   BONDAR_VERSION      - install a specific release tag instead of the latest
#   BONDAR_API_BASE     - GitHub API endpoint for resolving the latest release
#   BONDAR_DOWNLOAD_BASE - base URL the release assets are downloaded from
#
set -eu

REPO="iqbqioza/bondar"
API_BASE="${BONDAR_API_BASE:-https://api.github.com/repos/${REPO}/releases/latest}"
DOWNLOAD_BASE="${BONDAR_DOWNLOAD_BASE:-https://github.com/${REPO}/releases/download}"

# --- platform detection -----------------------------------------------------

case "$(uname -s)" in
    Linux)  os="linux" ;;
    Darwin) os="macos" ;;
    *)
        echo "error: unsupported platform: $(uname -s) (Linux and macOS only)" >&2
        exit 1
        ;;
esac

# Architecture detection. On macOS, prefer the native architecture: a shell
# running under Rosetta reports x86_64 from `uname -m` even on Apple Silicon,
# so ask the kernel directly when available.
arch=""
if [ "$os" = "macos" ] \
    && command -v sysctl >/dev/null 2>&1 \
    && [ "$(sysctl -n hw.optional.arm64 2>/dev/null)" = "1" ]; then
    arch="aarch64"
else
    case "$(uname -m)" in
        x86_64 | amd64)  arch="x86_64" ;;
        aarch64 | arm64) arch="aarch64" ;;
        *)
            echo "error: unsupported architecture: $(uname -m)" >&2
            exit 1
            ;;
    esac
fi

asset="bondar-${os}-${arch}"

if [ -z "${HOME:-}" ]; then
    echo "error: HOME is not set; cannot determine an install directory" >&2
    exit 1
fi

# --- tooling ----------------------------------------------------------------

download() { # url output_file
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        echo "error: neither curl nor wget is available" >&2
        return 1
    fi
}

on_path() { # dir
    case ":${PATH}:" in
        *":$1:"*) return 0 ;;
    esac
    return 1
}

# --- resolve the release tag -------------------------------------------------

tag="${BONDAR_VERSION:-}"
if [ -z "$tag" ]; then
    echo "Resolving the latest bondar release..."
    tmpjson=$(mktemp "${TMPDIR:-/tmp}/bondar-release.XXXXXX")
    trap 'rm -f "$tmpjson"' EXIT
    if ! download "$API_BASE" "$tmpjson"; then
        echo "error: failed to query the latest release from GitHub" >&2
        exit 1
    fi
    tag=$(sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' "$tmpjson" | head -n 1)
    rm -f "$tmpjson"
    trap - EXIT
    if [ -z "$tag" ]; then
        echo "error: could not parse the latest release tag from the GitHub API" >&2
        exit 1
    fi
fi
echo "Latest release: ${tag}"

# --- pick an install directory already on PATH -------------------------------

bindir=""
if [ -n "${XDG_BIN_HOME:-}" ] && on_path "${XDG_BIN_HOME}"; then
    bindir="${XDG_BIN_HOME}"
elif on_path "${HOME}/.local/bin"; then
    bindir="${HOME}/.local/bin"
elif on_path "${HOME}/bin"; then
    bindir="${HOME}/bin"
else
    # First user-writable directory already on PATH
    old_ifs=$IFS
    IFS=:
    for dir in $PATH; do
        if [ -n "$dir" ] && [ -d "$dir" ] && [ -w "$dir" ]; then
            bindir=$dir
            break
        fi
    done
    IFS=$old_ifs
fi

if [ -z "$bindir" ]; then
    # No writable dir on PATH: fall back to the XDG convention and explain
    bindir="${HOME}/.local/bin"
    echo "warning: no user-writable directory found on PATH" >&2
    echo "  installing to ${bindir} (add it to PATH with:" >&2
    echo "  export PATH=\"\${PATH}:${bindir}\")" >&2
fi
mkdir -p "$bindir"

target="${bindir}/bondar"

# --- overwrite confirmation -------------------------------------------------

if [ -e "$target" ]; then
    if [ -d "$target" ]; then
        echo "error: ${target} is a directory; refusing to replace it" >&2
        exit 1
    fi
    printf '%s already exists. Overwrite? [y/N] ' "$target"
    read -r answer || exit 1
    case "$answer" in
        y | Y | yes | YES)
            echo "Overwriting ${target}..."
            ;;
        *)
            echo "aborted: ${target} was not overwritten" >&2
            exit 1
            ;;
    esac
fi

# --- download, verify and install -------------------------------------------

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/bondar-install.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT

url="${DOWNLOAD_BASE}/${tag}/${asset}"
echo "Downloading ${url}..."
download "$url" "$tmpdir/bondar" || {
    echo "error: failed to download the bondar release asset" >&2
    exit 1
}

# Checksum verification (best effort; SHA256SUMS ships with the release)
if download "${DOWNLOAD_BASE}/${tag}/SHA256SUMS" "$tmpdir/SHA256SUMS" 2>/dev/null; then
    if command -v sha256sum >/dev/null 2>&1; then
        sum=$(sha256sum "$tmpdir/bondar" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then
        sum=$(shasum -a 256 "$tmpdir/bondar" | awk '{print $1}')
    else
        sum=""
    fi
    if [ -n "$sum" ] && ! grep -q "$sum" "$tmpdir/SHA256SUMS"; then
        echo "error: checksum verification failed for ${asset}" >&2
        exit 1
    fi
    echo "Checksum verified."
else
    echo "warning: SHA256SUMS not found for ${tag}; skipping checksum verification" >&2
fi

chmod +x "$tmpdir/bondar"
cp "$tmpdir/bondar" "$target" || {
    echo "error: failed to install to ${target}" >&2
    exit 1
}
chmod +x "$target"

echo "Installed bondar ${tag} to ${target}"

# --- smoke test -------------------------------------------------------------

if "$target" --version >/dev/null 2>&1; then
    echo "Verified: $("$target" --version)"
    if on_path "$bindir"; then
        echo "Done. Run 'bondar --help' to get started."
    else
        echo "Note: ${bindir} is not on your PATH; add it with:"
        echo "  export PATH=\"\${PATH}:${bindir}\""
        echo "  (consider adding it to your shell profile)"
    fi
else
    echo "warning: the installed binary could not be executed (${target})" >&2
    exit 1
fi