#!/usr/bin/env bash
set -euo pipefail

IMAGE_TAG="${IMAGE_TAG:-solo-new-api-rust:local}"
NAME="${NAME:-solo-rust-local}"
PORT="${PORT:-3006}"
DATA_DIR="${DATA_DIR:-/tmp/solo-deploy-data}"
SNAPSHOT="${SNAPSHOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/data/one-api-consistent.db}"

mkdir -p "$DATA_DIR"
if [ ! -f "$DATA_DIR/one-api.db" ]; then
    cp "$SNAPSHOT" "$DATA_DIR/one-api.db"
    echo "==> seeded $DATA_DIR/one-api.db from $SNAPSHOT"
fi

docker rm -f "$NAME" >/dev/null 2>&1 || true
docker run -d --name "$NAME" --platform linux/amd64 \
    --memory 450m \
    -p "$PORT:3000" \
    -v "$DATA_DIR:/data" \
    "$IMAGE_TAG" >/dev/null

echo "==> http://127.0.0.1:$PORT  (data: $DATA_DIR)"
