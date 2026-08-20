#!/usr/bin/env bash
set -euo pipefail

# ── Colours ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
info()    { echo -e "${GREEN}[INFO]${NC}  $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[ERROR]${NC} $*" >&2; exit 1; }

# ── Helpers ───────────────────────────────────────────────────────────────────
need() { command -v "$1" &>/dev/null || error "'$1' is required but not found. $2"; }

# ── Rust ──────────────────────────────────────────────────────────────────────
if ! command -v rustup &>/dev/null; then
    info "Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
    source "$HOME/.cargo/env"
else
    info "rustup already installed, updating..."
    rustup update stable
fi

need cargo "Rust/cargo not on PATH. Add \$HOME/.cargo/bin to your PATH."
source "$HOME/.cargo/env" 2>/dev/null || true

# ── Embedded target ───────────────────────────────────────────────────────────
# Change this to match your target (thumbv6m, thumbv7em-none-eabihf, etc.)
TARGET="${EMBED_TARGET:-thumbv7em-none-eabihf}"
info "Adding Rust target: $TARGET"
rustup target add "$TARGET"

# ── cargo-binstall ────────────────────────────────────────────────────────────
if ! command -v cargo-binstall &>/dev/null; then
    info "Installing cargo-binstall..."
    curl -L --proto '=https' --tlsv1.2 -sSf \
        https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh \
        | bash
else
    info "cargo-binstall already installed, skipping."
fi

# ── probe-rs ──────────────────────────────────────────────────────────────────
info "Installing probe-rs..."
cargo binstall probe-rs-tools --no-confirm

# Post-install: udev rules on Linux (needed to access USB probes without sudo)
if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    RULES_URL="https://probe.rs/files/69-probe-rs.rules"
    info "Installing udev rules for probe-rs (requires sudo)..."
    curl -fsSL "$RULES_URL" | sudo tee /etc/udev/rules.d/69-probe-rs.rules > /dev/null
    sudo udevadm control --reload-rules
    sudo udevadm trigger
    warn "You may need to log out and back in for udev rules to take effect."
fi

# ── flip-link ─────────────────────────────────────────────────────────────────
info "Installing flip-link..."
cargo install flip-link

# ── Done ──────────────────────────────────────────────────────────────────────
echo ""
info "All done! You may need to run: source \"\$HOME/.cargo/env\""
info "Installed tools:"
cargo binstall --version 2>/dev/null && true
probe-rs --version   2>/dev/null && true
flip-link --version  2>/dev/null && true