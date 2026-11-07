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
if ! docker build --platform linux/amd64 -f "$ROOT/deploy/Dockerfile" -t "$IMAGE_TAG" "$CONTEXT"; then
    # 镜像源 403 时退回本机缓存的同版本基座，避免整个构建中断。
    LOCAL_BASE="docker.1ms.run/debian:bookworm-slim"
    if ! docker image inspect "$LOCAL_BASE" >/dev/null 2>&1; then
        echo "==> docker build failed and $LOCAL_BASE is not cached" >&2
        exit 1
    fi
    echo "==> retrying with cached base $LOCAL_BASE"
    sed "1s|.*|FROM $LOCAL_BASE|" "$ROOT/deploy/Dockerfile" > "$CONTEXT/Dockerfile.local"
    docker build --platform linux/amd64 -f "$CONTEXT/Dockerfile.local" -t "$IMAGE_TAG" "$CONTEXT"
    rm -f "$CONTEXT/Dockerfile.local"
fi

echo "==> image ready: $IMAGE_TAG"
