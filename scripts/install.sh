#!/bin/sh
# Vlinder install script
# Usage: curl -fsSL https://vlinder.dev/install.sh | sh
#
# Installs the vlinder binary and bootstraps the ~/.vlinder directory.
# Safe to re-run for upgrades — existing config.toml is preserved.

set -eu

REPO="vlindercli/vlindercli"
INSTALL_DIR="/usr/local/bin"
VLINDER_DIR="${HOME}/.vlinder"

# --- Helpers ---

info() {
    printf '  %s\n' "$1"
}

warn() {
    printf '  [warn] %s\n' "$1"
}

error() {
    printf '  [error] %s\n' "$1" >&2
    exit 1
}

check_cmd() {
    command -v "$1" >/dev/null 2>&1
}

# --- Detect platform ---

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)  OS="linux" ;;
        Darwin) OS="darwin" ;;
        *)      error "Unsupported OS: $OS" ;;
    esac

    case "$ARCH" in
        x86_64|amd64)  ARCH="x86_64" ;;
        arm64|aarch64) ARCH="aarch64" ;;
        *)             error "Unsupported architecture: $ARCH" ;;
    esac

    case "${OS}-${ARCH}" in
        linux-x86_64)   TARGET="x86_64-unknown-linux-gnu" ;;
        darwin-x86_64)  TARGET="x86_64-apple-darwin" ;;
        darwin-aarch64) TARGET="aarch64-apple-darwin" ;;
        *)              error "Unsupported platform: ${OS}-${ARCH}" ;;
    esac

    info "Detected platform: ${TARGET}"
}

# --- Resolve version ---

resolve_version() {
    if [ -n "${VLINDER_VERSION:-}" ]; then
        VERSION="$VLINDER_VERSION"
        info "Using specified version: ${VERSION}"
        return
    fi

    info "Fetching latest release..."
    VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' \
        | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')

    if [ -z "$VERSION" ]; then
        error "Could not determine latest version. Set VLINDER_VERSION to install a specific version."
    fi

    info "Latest version: ${VERSION}"
}

# --- Download and install binary ---

install_binary() {
    ARCHIVE="vlinder-${TARGET}.tar.gz"
    URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}"

    info "Downloading ${URL}..."
    TMPDIR=$(mktemp -d)
    trap 'rm -rf "$TMPDIR"' EXIT

    curl -fsSL "$URL" -o "${TMPDIR}/${ARCHIVE}"
    tar xzf "${TMPDIR}/${ARCHIVE}" -C "$TMPDIR"

    if [ ! -f "${TMPDIR}/vlinder" ]; then
        error "Archive did not contain vlinder binary"
    fi

    # Install to INSTALL_DIR — may need sudo
    if [ -w "$INSTALL_DIR" ]; then
        cp "${TMPDIR}/vlinder" "${INSTALL_DIR}/vlinder"
        chmod +x "${INSTALL_DIR}/vlinder"
    else
        info "Installing to ${INSTALL_DIR} (requires sudo)..."
        sudo cp "${TMPDIR}/vlinder" "${INSTALL_DIR}/vlinder"
        sudo chmod +x "${INSTALL_DIR}/vlinder"
    fi

    info "Installed vlinder to ${INSTALL_DIR}/vlinder"
}

# --- Bootstrap directory structure ---

bootstrap_dirs() {
    mkdir -p "${VLINDER_DIR}/agents"
    mkdir -p "${VLINDER_DIR}/conversations"
    mkdir -p "${VLINDER_DIR}/logs"
    mkdir -p "${VLINDER_DIR}/models"
    info "Directory structure ready at ${VLINDER_DIR}"
}

# --- Write default config ---

write_config() {
    CONFIG_FILE="${VLINDER_DIR}/config.toml"

    if [ -f "$CONFIG_FILE" ]; then
        info "Config exists at ${CONFIG_FILE} — skipping (preserving your settings)"
        return
    fi

    cat > "$CONFIG_FILE" << 'EOF'
# Vlinder configuration
# Docs: https://github.com/vlindercli/vlindercli

[logging]
level = "info"

[ollama]
endpoint = "http://localhost:11434"

[queue]
backend = "nats"
nats_url = "nats://localhost:4222"

[distributed]
enabled = true
registry_addr = "http://127.0.0.1:9090"

[distributed.workers]
registry = 1

[distributed.workers.agent]
container = 1

[distributed.workers.inference]
ollama = 1

[distributed.workers.embedding]
ollama = 1

[distributed.workers.storage.object]
sqlite = 1

[distributed.workers.storage.vector]
sqlite = 1
EOF

    info "Wrote default config to ${CONFIG_FILE}"
}

# --- Check prerequisites ---

check_prerequisites() {
    printf '\n  Prerequisites:\n'
    MISSING_REQUIRED=0

    if check_cmd nats-server; then
        info "  nats-server  ... found"
    else
        warn "  nats-server  ... NOT FOUND (required)"
        MISSING_REQUIRED=1
    fi

    if check_cmd podman; then
        info "  podman       ... found"
    else
        warn "  podman       ... NOT FOUND (required)"
        MISSING_REQUIRED=1
    fi

    if check_cmd ollama; then
        info "  ollama       ... found"
    else
        warn "  ollama       ... not found (optional — needed for LLM inference)"
    fi

    if [ "$MISSING_REQUIRED" -eq 1 ]; then
        printf '\n'
        warn "Some required prerequisites are missing."
        warn "Install them before running 'vlinder daemon'."
    fi
}

# --- Main ---

main() {
    printf '\n  Vlinder Installer\n'
    printf '  =================\n\n'

    detect_platform
    resolve_version
    install_binary
    bootstrap_dirs
    write_config
    check_prerequisites

    printf '\n  Done! Next steps:\n'
    printf '  1. Start NATS:          nats-server --jetstream\n'
    printf '  2. Start Ollama:        ollama serve\n'
    printf '  3. Pull a model:        ollama pull phi3\n'
    printf '  4. Start the daemon:    vlinder daemon\n'
    printf '  5. Register a model:    vlinder model add phi3\n'
    printf '  6. Try it out:          vlinder support\n'
    printf '\n'
}

main
