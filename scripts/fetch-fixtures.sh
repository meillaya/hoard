#!/usr/bin/env bash
#
# Fetch the real GGUF fixtures used by the integration tests.
#
# The files are pinned to a Hugging Face commit and verified by SHA-256. They
# are not committed (fixtures/real/ is gitignored); this script is the source
# of truth for their provenance.

set -euo pipefail

readonly REPO="ggml-org/models"
readonly REVISION="499bc8821c6b12b4e53c5bffcb21ec206f212d81"
readonly BASE_URL="https://huggingface.co/${REPO}/resolve/${REVISION}/tinyllamas"

# name, sha256 of the file contents (HF LFS oid)
readonly FILES=(
  "stories260K.gguf 270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d"
  "stories260K-be.gguf 276d6f9d9050e79670a113492331dc52b3568690b03f5e633521cbed020bfc1c"
)

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="${repo_root}/fixtures/real"
force=0
if [[ "${1:-}" == "--force" ]]; then
  force=1
elif [[ $# -gt 0 ]]; then
  echo "usage: $0 [--force]" >&2
  exit 2
fi

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | awk '{print $1}'; }
else
  sha256() { shasum -a 256 "$1" | awk '{print $1}'; }
fi

mkdir -p "$dest"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

for entry in "${FILES[@]}"; do
  read -r name expected <<<"${entry}"
  out="${dest}/${name}"

  if [[ -f "$out" && "$force" -eq 0 ]]; then
    actual="$(sha256 "$out")"
    if [[ "$actual" == "$expected" ]]; then
      echo "ok: ${name} (cached, sha256 verified)"
      continue
    fi
    echo "warning: ${name} has the wrong sha256; re-downloading" >&2
  fi

  echo "fetching ${name}" >&2
  if ! curl -fsSL --retry 3 --retry-delay 2 --max-time 300 \
    -o "${tmpdir}/${name}" "${BASE_URL}/${name}"; then
    echo "error: failed to download ${BASE_URL}/${name}" >&2
    exit 1
  fi

  actual="$(sha256 "${tmpdir}/${name}")"
  if [[ "$actual" != "$expected" ]]; then
    echo "error: sha256 mismatch for ${name}" >&2
    echo "  expected: ${expected}" >&2
    echo "  actual:   ${actual}" >&2
    exit 1
  fi

  mv -f "${tmpdir}/${name}" "$out"
  echo "ok: ${name} (sha256 verified)"
done

echo "fixtures in ${dest}" >&2
