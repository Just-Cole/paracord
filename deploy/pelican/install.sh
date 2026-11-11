#!/bin/bash
# Paracord install script for Pelican. Runs in the rust:1-bookworm installer
# container with the server's files mounted at /mnt/server.
#
# It clones the repo, builds the web client and a static Linux binary, and
# installs the binary into the server. Your data lives in MySQL, which this
# script never touches, so "Reinstall" is how you update.
#
# The source and build cache live in /mnt/server/.build (about 1.5 GB). They
# stay between reinstalls so updates only recompile what changed. /tmp is not
# used because Pelican mounts it as a small in-memory folder.
set -euo pipefail

REPO="${GIT_ADDRESS:-https://github.com/Just-Cole/paracord.git}"
BRANCH="${BRANCH:-main}"
TRUNK_VERSION="0.22.1"
WORK=/mnt/server/.build
SRC="$WORK/src"
export CARGO_TARGET_DIR="$WORK/target"
export HOME="$WORK/home"   # Trunk caches wasm-bindgen and wasm-opt here.
mkdir -p "$WORK" "$HOME"

case "$(uname -m)" in
    x86_64) TARGET=x86_64-unknown-linux-musl ;;
    aarch64) TARGET=aarch64-unknown-linux-musl ;;
    *) echo "Unsupported CPU: $(uname -m)"; exit 1 ;;
esac

echo "==> Disk space for the build"
df -h /mnt/server | tail -1

echo "==> Installing build tools"
apt-get update -qq
apt-get install -y -qq --no-install-recommends git musl-tools ca-certificates curl >/dev/null
rustup target add wasm32-unknown-unknown "$TARGET"

# Trunk: use the static (musl) release so it runs whatever the image's glibc is.
TRUNK="$WORK/bin/trunk"
if ! "$TRUNK" --version >/dev/null 2>&1; then
    mkdir -p "$WORK/bin"
    TRUNK_URL="https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-$(uname -m)-unknown-linux-musl.tar.gz"
    curl -fsSL "$TRUNK_URL" | tar -xz -C "$WORK/bin" trunk || true
    if ! "$TRUNK" --version >/dev/null 2>&1; then
        echo "Prebuilt trunk didn't run; compiling it (takes a few minutes)"
        cargo install trunk --locked --version "$TRUNK_VERSION" --root "$WORK"
    fi
fi
"$TRUNK" --version

echo "==> Fetching ${REPO} (${BRANCH})"
if [ -d "$SRC/.git" ] && git -C "$SRC" remote get-url origin | grep -qxF "$REPO"; then
    git -C "$SRC" fetch --depth 1 origin "$BRANCH"
    git -C "$SRC" reset --hard FETCH_HEAD
    git -C "$SRC" clean -fdx -e web/dist
else
    rm -rf "$SRC"
    git clone --depth 1 --branch "$BRANCH" "$REPO" "$SRC"
fi
cd "$SRC"

# Keep memory use modest; install containers often get 1-2 GB.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export CARGO_PROFILE_RELEASE_LTO=thin
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16

echo "==> Building the web client"
(cd web && "$TRUNK" build --release)

echo "==> Building the server"
cargo build --release --locked -p paracord-server --target "$TARGET"

echo "==> Installing"
install -m 755 "$CARGO_TARGET_DIR/$TARGET/release/paracord" /mnt/server/paracord
git rev-parse --short HEAD > /mnt/server/VERSION
echo "Paracord $(cat /mnt/server/VERSION) installed."
df -h /mnt/server | tail -1
