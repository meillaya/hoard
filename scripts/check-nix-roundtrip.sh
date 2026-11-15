#!/usr/bin/env bash
#
# Build the example model artifact, push its FODs to a local file cache, delete
# them from the store, and rebuild with networking disabled. The two build
# outputs must be identical.
#
# Requires `hoard fetch` in the packaged binary.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
example_dir="${repo_root}/examples/nix"
cache_dir="$(mktemp -d)"
trap 'rm -rf "$cache_dir"' EXIT

local_hoard="git+file:${repo_root}"

build() {
  (cd "$example_dir" && nix build .#default \
    --override-input hoard "$local_hoard" \
    --no-write-lock-file --no-link --print-out-paths "$@")
}

before="$(build)"
fods="$(nix-store --query --references "$before")"
if [[ -z "$fods" ]]; then
  echo "error: the model output has no fixed-output references" >&2
  exit 1
fi
mapfile -t fod_paths <<< "$fods"

nix copy --to "file://${cache_dir}" "${fod_paths[@]}"
rm -f "${example_dir}/result"
if [[ "${HOARD_CACHE_ONLY:-0}" == "1" ]]; then
  echo "cache population ok; skipped store deletion (HOARD_CACHE_ONLY=1)" >&2
  echo "cache: ${cache_dir}" >&2
  echo "farm:  ${before}" >&2
  exit 0
fi

delete_paths() {
  local _attempt
  for _attempt in 1 2 3 4 5; do
    if nix-store --delete "$@" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  return 1
}

# Delete the farm first so the FODs are no longer referenced, then the FODs.
if ! delete_paths "$before" || ! delete_paths "${fod_paths[@]}"; then
  echo "error: cannot delete the store paths; a clean store is required for this test" >&2
  exit 1
fi

after="$(build --offline --option substituters "file://${cache_dir}" --option require-sigs false)"

if [[ "$before" != "$after" ]]; then
  echo "error: offline build produced a different path" >&2
  echo "  before: ${before}" >&2
  echo "  after:  ${after}" >&2
  exit 1
fi

echo "round-trip ok: ${after}"
