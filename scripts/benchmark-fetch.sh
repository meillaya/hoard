#!/usr/bin/env bash
#
# Time one pinned Hugging Face file over each transport. The default file is
# small; pass a repository path and spec to benchmark a real model.
#
# Usage:
#   scripts/benchmark-fetch.sh [SPEC] [FILE]
#
# Set HOARD_BIN to use a binary other than `hoard` on PATH.

set -euo pipefail

spec="${1:-hf:ggml-org/models@499bc8821c6b12b4e53c5bffcb21ec206f212d81}"
file="${2:-tinyllamas/stories260K.gguf}"
hoard_bin="${HOARD_BIN:-hoard}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

if ! command -v "$hoard_bin" >/dev/null 2>&1; then
  echo "error: $hoard_bin is not on PATH; build it or set HOARD_BIN" >&2
  exit 1
fi

export HOARD_STORE="$work/store"

for transport in auto xet http; do
  rm -rf "$work/out" "$work/manifest.json"
  if command -v sha256sum >/dev/null 2>&1; then
    :
  fi
  TIMEFORMAT="${transport} %3R s"
  time "$hoard_bin" fetch "$spec" \
    --file "$file" \
    --transport "$transport" \
    --manifest-out "$work/manifest.json" \
    --out "$work/out"
done

echo "manifest: $work/manifest.json"
sha256sum "$work/out/$(dirname "$file")"/* 2>/dev/null || sha256sum "$work/out"/* 2>/dev/null || true
