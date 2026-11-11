#!/bin/bash
# Paracord install script for Pelican. Runs in the rust:1-bookworm installer
# container with the server's files mounted at /mnt/server.
#
# It clones the repo, builds the web client and a static Linux binary, and
# copies only the binary into the server. data/ (the database) is never touched,
# so "Reinstall" is how you update.
set -euo pipefail

REPO="${GIT_ADDRESS:-https://github.com/Just-Cole/paracord.git}"
BRANCH="${BRANCH:-main}"
TRUNK_VERSION="0.22.1"
BUILD=/tmp/paracord-build

case "$(uname -m)" in
    x86_64) TARGET=x86_64-unknown-linux-musl ;;
    aarch64) TARGET=aarch64-unknown-linux-musl ;;
    *) echo "Unsupported CPU: $(uname -m)"; exit 1 ;;
esac

echo "==> Installing build tools"
apt-get update -qq
apt-get install -y -qq --no-install-recommends git musl-tools ca-certificates curl >/dev/null
rustup target add wasm32-unknown-unknown "$TARGET"

if ! command -v trunk >/dev/null; then
    TRUNK_URL="https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-$(uname -m)-unknown-linux-gnu.tar.gz"
    if curl -fsSL "$TRUNK_URL" | tar -xz -C /usr/local/bin trunk 2>/dev/null; then
        echo "Downloaded trunk ${TRUNK_VERSION}"
    else
        echo "No prebuilt trunk for this machine; compiling it (takes a few minutes)"
        cargo install trunk --locked --version "$TRUNK_VERSION"
    fi
fi

echo "==> Fetching ${REPO} (${BRANCH})"
rm -rf "$BUILD"
git clone --depth 1 --branch "$BRANCH" "$REPO" "$BUILD"
cd "$BUILD"

# Keep memory use modest; install containers often get 1-2 GB.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export CARGO_PROFILE_RELEASE_LTO=thin
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16

echo "==> Building the web client"
(cd web && trunk build --release)

echo "==> Building the server"
cargo build --release --locked -p paracord-server --target "$TARGET"

echo "==> Installing"
mkdir -p /mnt/server/data
install -m 755 "target/$TARGET/release/paracord" /mnt/server/paracord
git rev-parse --short HEAD > /mnt/server/VERSION
rm -rf "$BUILD"
echo "Paracord $(cat /mnt/server/VERSION) installed."
