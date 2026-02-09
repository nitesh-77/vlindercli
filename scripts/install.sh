#!/bin/sh
# Vlinder install script
# Usage: curl -fsSL https://vlindercli.dev/install.sh | sh
#
# Installs the vlinder binary, bootstraps ~/.vlinder, and sets up
# the daemon as a system service. Safe to re-run for upgrades.

set -eu

REPO="vlindercli/vlindercli"
INSTALL_DIR="/usr/local/bin"
VLINDER_DIR="${HOME}/.vlinder"

# --- Helpers ---

info() {
    printf '  %s\n' "$1"
}

error() {
    printf '\n  [error] %s\n\n' "$1" >&2
    exit 1
}

check_cmd() {
    command -v "$1" >/dev/null 2>&1
}

ok() {
    printf '  \342\234\223 %-12s %s\n' "$1" "$2"
}

fail() {
    printf '  \342\234\227 %-12s %s\n' "$1" "$2"
}

skip() {
    printf '  - %-12s %s\n' "$1" "$2"
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

    ok "Platform" "${TARGET}"
}

# --- Resolve version ---

resolve_version() {
    if [ -n "${VLINDER_VERSION:-}" ]; then
        VERSION="$VLINDER_VERSION"
    else
        VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' \
            | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')

        if [ -z "$VERSION" ]; then
            error "Could not determine latest version. Set VLINDER_VERSION to install a specific version."
        fi
    fi

    ok "Version" "${VERSION}"
}

# --- Download and install binary ---

install_binary() {
    ARCHIVE="vlinder-${TARGET}.tar.gz"
    URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}"

    TMPDIR=$(mktemp -d)
    trap 'rm -rf "$TMPDIR"' EXIT

    curl -fsSL "$URL" -o "${TMPDIR}/${ARCHIVE}"
    tar xzf "${TMPDIR}/${ARCHIVE}" -C "$TMPDIR"

    if [ ! -f "${TMPDIR}/vlinder" ]; then
        error "Archive did not contain vlinder binary"
    fi

    if [ -w "$INSTALL_DIR" ]; then
        cp "${TMPDIR}/vlinder" "${INSTALL_DIR}/vlinder"
        chmod +x "${INSTALL_DIR}/vlinder"
    else
        sudo cp "${TMPDIR}/vlinder" "${INSTALL_DIR}/vlinder"
        sudo chmod +x "${INSTALL_DIR}/vlinder"
    fi

    ok "Binary" "${INSTALL_DIR}/vlinder"
}

# --- Bootstrap directory structure ---

bootstrap_dirs() {
    mkdir -p "${VLINDER_DIR}/agents"
    mkdir -p "${VLINDER_DIR}/conversations"
    mkdir -p "${VLINDER_DIR}/logs"
    mkdir -p "${VLINDER_DIR}/registry"
    ok "Data dir" "${VLINDER_DIR}"
}

# --- Write default config ---

write_config() {
    CONFIG_FILE="${VLINDER_DIR}/config.toml"

    if [ -f "$CONFIG_FILE" ]; then
        ok "Config" "${CONFIG_FILE} (preserved)"
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

    ok "Config" "${CONFIG_FILE}"
}

# --- Check prerequisites ---

check_prerequisites() {
    HAVE_NATS=false
    HAVE_PODMAN=false
    HAVE_OLLAMA=false
    MISSING_REQUIRED=false

    if check_cmd nats-server; then
        HAVE_NATS=true
        ok "nats-server" "found"
    else
        fail "nats-server" "not found"
    fi

    if check_cmd podman; then
        HAVE_PODMAN=true
        ok "podman" "found"
    else
        fail "podman" "not found"
    fi

    if check_cmd ollama; then
        HAVE_OLLAMA=true
        ok "ollama" "found"
    else
        skip "ollama" "not found (optional, needed for LLM inference)"
    fi

    if [ "$HAVE_NATS" = false ] || [ "$HAVE_PODMAN" = false ]; then
        MISSING_REQUIRED=true
    fi
}

# --- NATS config and service ---

NATS_CONF="${VLINDER_DIR}/nats.conf"
NATS_DATA="${VLINDER_DIR}/nats-data"

write_nats_config() {
    if [ -f "$NATS_CONF" ]; then
        ok "NATS config" "${NATS_CONF} (preserved)"
        return
    fi

    mkdir -p "$NATS_DATA"

    cat > "$NATS_CONF" << EOF
# NATS configuration for Vlinder
# JetStream is required for message durability.

listen: 0.0.0.0:4222

jetstream {
    store_dir: ${NATS_DATA}
}
EOF

    ok "NATS config" "${NATS_CONF}"
}

has_existing_nats_service() {
    if [ "$OS" = "darwin" ]; then
        # Check for brew-managed or other launchd NATS services
        launchctl list 2>/dev/null | grep -q nats && return 0
        [ -f "${HOME}/Library/LaunchAgents/homebrew.mxcl.nats-server.plist" ] && return 0
    else
        # Check for systemd NATS services
        systemctl --user is-enabled nats-server.service >/dev/null 2>&1 && return 0
        systemctl is-enabled nats-server.service >/dev/null 2>&1 && return 0
    fi
    return 1
}

install_nats_service() {
    if has_existing_nats_service; then
        ok "NATS" "existing service detected"
        info "              Ensure JetStream is enabled in your NATS config."
        return
    fi

    if [ "$OS" = "darwin" ]; then
        install_nats_launchd
    else
        install_nats_systemd
    fi
}

install_nats_launchd() {
    NATS_PLIST_DIR="${HOME}/Library/LaunchAgents"
    NATS_PLIST="${NATS_PLIST_DIR}/dev.vlinder.nats.plist"
    NATS_LOG_DIR="${HOME}/Library/Logs/vlinder"
    NATS_BIN=$(command -v nats-server)

    mkdir -p "$NATS_PLIST_DIR"
    mkdir -p "$NATS_LOG_DIR"

    if [ -f "$NATS_PLIST" ]; then
        launchctl bootout "gui/$(id -u)" "$NATS_PLIST" 2>/dev/null || true
    fi

    cat > "$NATS_PLIST" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>dev.vlinder.nats</string>
    <key>ProgramArguments</key>
    <array>
        <string>${NATS_BIN}</string>
        <string>-c</string>
        <string>${NATS_CONF}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>${NATS_LOG_DIR}/nats.log</string>
    <key>StandardErrorPath</key>
    <string>${NATS_LOG_DIR}/nats.err</string>
</dict>
</plist>
PLIST

    launchctl bootstrap "gui/$(id -u)" "$NATS_PLIST"
    ok "NATS" "started with JetStream (launchd)"
}

install_nats_systemd() {
    NATS_UNIT_DIR="${HOME}/.config/systemd/user"
    NATS_UNIT="${NATS_UNIT_DIR}/vlinder-nats.service"
    NATS_BIN=$(command -v nats-server)

    mkdir -p "$NATS_UNIT_DIR"

    cat > "$NATS_UNIT" << UNIT
[Unit]
Description=NATS server for Vlinder (JetStream enabled)
Before=vlinder.service

[Service]
ExecStart=${NATS_BIN} -c ${NATS_CONF}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
UNIT

    systemctl --user daemon-reload
    systemctl --user enable --now vlinder-nats.service
    ok "NATS" "started with JetStream (systemd)"
}

# --- Install vlinder daemon service ---

install_service() {
    if [ "$OS" = "darwin" ]; then
        install_launchd_service
    else
        install_systemd_service
    fi
}

install_launchd_service() {
    PLIST_DIR="${HOME}/Library/LaunchAgents"
    PLIST_FILE="${PLIST_DIR}/dev.vlinder.daemon.plist"
    LOG_DIR="${HOME}/Library/Logs/vlinder"

    mkdir -p "$PLIST_DIR"
    mkdir -p "$LOG_DIR"

    if [ -f "$PLIST_FILE" ]; then
        launchctl bootout "gui/$(id -u)" "$PLIST_FILE" 2>/dev/null || true
    fi

    cat > "$PLIST_FILE" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>dev.vlinder.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>${INSTALL_DIR}/vlinder</string>
        <string>daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>${LOG_DIR}/daemon.log</string>
    <key>StandardErrorPath</key>
    <string>${LOG_DIR}/daemon.err</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>${HOME}</string>
    </dict>
</dict>
</plist>
PLIST

    launchctl bootstrap "gui/$(id -u)" "$PLIST_FILE"
    ok "Service" "started (launchd)"
}

install_systemd_service() {
    UNIT_DIR="${HOME}/.config/systemd/user"
    UNIT_FILE="${UNIT_DIR}/vlinder.service"

    mkdir -p "$UNIT_DIR"

    cat > "$UNIT_FILE" << UNIT
[Unit]
Description=Vlinder daemon
After=network.target nats-server.service

[Service]
ExecStart=${INSTALL_DIR}/vlinder daemon
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
UNIT

    systemctl --user daemon-reload
    systemctl --user enable --now vlinder.service
    ok "Service" "started (systemd)"
}

# --- Bootstrap support fleet ---

# TODO: Replace with real ghcr.io URL once support fleet repo CI publishes images.
SUPPORT_FLEET_IMAGE="ghcr.io/vlindercli/support-fleet:latest"

wait_for_daemon() {
    # Give the daemon a moment to start accepting connections.
    ATTEMPTS=0
    while [ "$ATTEMPTS" -lt 10 ]; do
        if "${INSTALL_DIR}/vlinder" model list >/dev/null 2>&1; then
            return 0
        fi
        ATTEMPTS=$((ATTEMPTS + 1))
        sleep 1
    done
    return 1
}

bootstrap_support() {
    if ! wait_for_daemon; then
        fail "Support" "daemon did not become ready"
        return
    fi

    if [ "$HAVE_OLLAMA" = true ]; then
        printf '\n  Pulling phi3 model (this may take a few minutes)...\n\n'
        ollama pull phi3 || true
        printf '\n'
        "${INSTALL_DIR}/vlinder" model add phi3 >/dev/null 2>&1 || true
        ok "Model" "phi3 pulled and registered"
    else
        skip "Model" "skipped (Ollama not available)"
    fi

    "${INSTALL_DIR}/vlinder" fleet run "${SUPPORT_FLEET_IMAGE}" >/dev/null 2>&1 || true
    ok "Support" "fleet deployed"
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

    if [ "$MISSING_REQUIRED" = true ]; then
        printf '\n  Vlinder is installed but cannot start yet.\n'
        printf '  NATS and Podman are required to run the daemon.\n'
        printf '\n  Install the missing dependencies, then re-run this script:\n\n'

        if [ "$HAVE_NATS" = false ]; then
            if [ "$OS" = "darwin" ]; then
                info "  brew install nats-server"
            else
                info "  curl -fsSL https://get.nats.io | sh"
            fi
        fi

        if [ "$HAVE_PODMAN" = false ]; then
            if [ "$OS" = "darwin" ]; then
                info "  brew install podman"
            else
                info "  sudo apt install -y podman     # Debian/Ubuntu"
                info "  sudo dnf install -y podman     # Fedora/RHEL"
            fi
        fi

        if [ "$HAVE_OLLAMA" = false ]; then
            info "  curl -fsSL https://ollama.com/install.sh | sh    (optional)"
        fi

        printf '\n  Then re-run:\n\n'
        info "  curl -fsSL https://vlindercli.dev/install.sh | sh"
        printf '\n'
        exit 0
    fi

    write_nats_config
    install_nats_service
    install_service
    bootstrap_support

    printf '\n  Vlinder is installed and running.\n'

    if [ "$HAVE_OLLAMA" = false ]; then
        printf '\n  Ollama is not installed. Agents that use LLMs will not work.\n'
        printf '  To add it later:\n\n'
        info "  curl -fsSL https://ollama.com/install.sh | sh"
        info "  ollama pull phi3"
        info "  vlinder model add phi3"
    fi

    printf '\n  Try it:\n\n'
    info "  vlinder support"
    printf '\n'
}

main
