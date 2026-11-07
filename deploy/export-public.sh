#!/usr/bin/env bash
# 导出一份不含私有开发文档的公开快照。
# 私有文档自拆分提交起已不被跟踪；旧提交里的历史副本按需另行处理。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT/deploy/solo-new-api-rust-public.tar.gz}"

git -C "$ROOT" archive --format=tar.gz --prefix=solo-new-api-rust/ -o "$OUT" HEAD

echo "==> public snapshot: $OUT"
tar -tzf "$OUT" | grep -E 'docs/dev|REWRITE_SCOPE|superpowers' && {
    echo "ERROR: private docs leaked into the snapshot" >&2
    exit 1
} || true
echo "==> no private docs in snapshot"
