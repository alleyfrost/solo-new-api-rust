#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE_TAG="${IMAGE_TAG:-solo-new-api-rust:local}"
TARGET_DIR="${TARGET_DIR:-/tmp/solo-target-amd64}"
CONTEXT="$ROOT/deploy/context"

echo "==> building linux/amd64 release"
docker run --rm --platform linux/amd64 \
    -v "$ROOT":/src \
    -v "$HOME/docker/cargo-home/registry":/usr/local/cargo/registry \
    -v "$HOME/docker/cargo-home/git":/usr/local/cargo/git \
    -v "$TARGET_DIR":/target \
    -w /src \
    -e CARGO_TARGET_DIR=/target \
    -e RUSTUP_TOOLCHAIN=1.96.1 \
    rust:1.96-bookworm \
    cargo build --release --locked

echo "==> building frontend"
(cd "$ROOT/web" && bun run build)

echo "==> assembling runtime image $IMAGE_TAG"
rm -rf "$CONTEXT"
mkdir -p "$CONTEXT"
cp "$TARGET_DIR/release/solo-new-api-rust" "$CONTEXT/solo-new-api-rust"
cp -R "$ROOT/web/dist" "$CONTEXT/web-dist"
docker build --platform linux/amd64 -f "$ROOT/deploy/Dockerfile" -t "$IMAGE_TAG" "$CONTEXT"

echo "==> image ready: $IMAGE_TAG"
