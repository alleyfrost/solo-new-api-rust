#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE_TAG="${IMAGE_TAG:-solo-new-api-rust:local}"
ARCHIVE="${ARCHIVE:-$ROOT/deploy/solo-new-api-rust-amd64.tar.gz}"

docker save "$IMAGE_TAG" | gzip -9 > "$ARCHIVE"
ls -lh "$ARCHIVE"
