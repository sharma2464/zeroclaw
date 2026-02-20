#!/usr/bin/env bash
set -euo pipefail

# ── ZeroClaw Termux Bootstrap ───────────────────────────────────
# Automates dependencies, bypasses rustup panics, and builds the
# release binary using the native Termux toolchain.

info() { echo -e "\033[1;34m==>\033[0m $*"; }
error() { echo -e "\033[1;31merror:\033[0m $*" >&2; }

# 1. Update system packages
info "Updating system packages..."
pkg update && pkg upgrade -y

# 2. Install native build dependencies (bypass rustup)
info "Installing native build dependencies..."
pkg install -y rust binutils-is-llvm pkg-config curl git

# 3. Locate native binaries (Termux-specific paths)
TERMUX_CARGO="/data/data/com.termux/files/usr/bin/cargo"
TERMUX_RUSTC="/data/data/com.termux/files/usr/bin/rustc"

if [[ ! -f "$TERMUX_CARGO" ]]; then
    error "Native cargo not found at $TERMUX_CARGO. Please run: pkg install rust"
    exit 1
fi

# 4. Build ZeroClaw (bypass rustup environment)
info "Building ZeroClaw release binary..."
RUSTUP_TOOLCHAIN="" 
RUSTC="$TERMUX_RUSTC" 
"$TERMUX_CARGO" build --release --locked

# 5. Verify build
BINARY="./target/release/zeroclaw"
if [[ -f "$BINARY" ]]; then
    info "✅ ZeroClaw successfully built at $BINARY"
    "$BINARY" --version
else
    error "❌ Build failed. Please check build_output.log"
    exit 1
fi

cat <<'DONE'

🚀 ZeroClaw is ready on Termux!

Next steps:
  ./target/release/zeroclaw onboard
  ./target/release/zeroclaw status
DONE
