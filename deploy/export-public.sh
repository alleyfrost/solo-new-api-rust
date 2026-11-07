#!/usr/bin/env bash
# 导出一份不含私有开发文档的公开快照。
# 注意：这只是 HEAD 的干净树；当前 git 历史仍包含被移除的私有文档，
# 公开仓库应在此快照上重新初始化，不要直接推送本仓库历史。
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
