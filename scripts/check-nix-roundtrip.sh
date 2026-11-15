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

# shellcheck disable=SC2086
nix copy --to "file://${cache_dir}" ${fods}
rm -f "${example_dir}/result"
# Delete the farm first so the FODs are no longer referenced, then the FODs.
nix-store --delete "$before"
# shellcheck disable=SC2086
nix-store --delete ${fods}

after="$(build --offline --option substituters "file://${cache_dir}" --option require-sigs false)"

if [[ "$before" != "$after" ]]; then
  echo "error: offline build produced a different path" >&2
  echo "  before: ${before}" >&2
  echo "  after:  ${after}" >&2
  exit 1
fi

echo "round-trip ok: ${after}"
