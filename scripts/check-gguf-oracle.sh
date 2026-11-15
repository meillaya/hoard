#!/usr/bin/env bash
#
# Differential oracle: compare `hoard-gguf inspect --json` with llama.cpp's
# gguf-py reference reader at a pinned commit.
#
# The pinned commit is defined once, below. The script downloads the gguf-py
# directory of that exact tree (never vendoring llama.cpp into the repository),
# builds the local `hoard-gguf` binary, and runs `scripts/gguf_oracle.py` over
# each GGUF file. Any real difference fails the script with a per-field report;
# files are never skipped silently.
#
# Usage:
#   scripts/check-gguf-oracle.sh [GGUF...]
#
# With no arguments it checks the committed synthetic fixtures plus the real
# stories260K fixture (fetching that file with scripts/fetch-fixtures.sh if it
# is absent).
#
# Environment:
#   HOARD_ORACLE_CACHE            cache directory for the llama.cpp tree
#                                (default: <repo>/.cache/gguf-oracle)
#   HOARD_ORACLE_LLAMA_CPP_DIR    use an existing llama.cpp tree instead of
#                                downloading one (must contain gguf-py/)
#   HOARD_ORACLE_PYTHON           Python interpreter that can import numpy
#                                (default: python3; falls back to
#                                `nix shell nixpkgs#python3Packages.numpy`)
#   CARGO_HOME                    cargo home for the build (default: <repo>/.cargo-home)
#
set -euo pipefail

# The one place the reference commit is pinned.
readonly LLAMA_CPP_REPO="ggml-org/llama.cpp"
readonly LLAMA_CPP_COMMIT="de32b268c4ad55d45f3b7b16c029f4e077f4ba74"
readonly LLAMA_CPP_TARBALL="https://codeload.github.com/${LLAMA_CPP_REPO}/tar.gz/${LLAMA_CPP_COMMIT}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cache_dir="${HOARD_ORACLE_CACHE:-${repo_root}/.cache/gguf-oracle}"
tree_dir="${HOARD_ORACLE_LLAMA_CPP_DIR:-${cache_dir}/llama.cpp-${LLAMA_CPP_COMMIT}}"
python_cmd=("${HOARD_ORACLE_PYTHON:-python3}")

usage() {
  sed -n '2,34p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

# 1. Reference tree: reuse a cached/extracted tree, otherwise download the
#    gguf-py subtree of the pinned commit.
reference_complete() {
  local tree="$1"
  for required in __init__.py constants.py quants.py gguf_reader.py; do
    if [[ ! -f "${tree}/gguf-py/gguf/${required}" ]]; then
      return 1
    fi
  done
  return 0
}
if reference_complete "${tree_dir}"; then
  echo "reference tree: ${tree_dir} (cached)" >&2
else
  for tool in curl tar; do
    if ! command -v "${tool}" >/dev/null 2>&1; then
      echo "error: ${tool} is required to obtain llama.cpp@${LLAMA_CPP_COMMIT}" >&2
      exit 2
    fi
  done
  mkdir -p "${cache_dir}"
  download_dir="$(mktemp -d "${cache_dir}/download.XXXXXX")"
  trap 'rm -rf "${download_dir}"' EXIT
  echo "fetching llama.cpp@${LLAMA_CPP_COMMIT} (gguf-py subtree) into ${tree_dir}" >&2
  curl -fsSL --retry 3 --retry-delay 5 --max-time 1800 \
    -o "${download_dir}/llama.cpp.tar.gz" "${LLAMA_CPP_TARBALL}"
  mkdir -p "${download_dir}/tree"
  tar -xzf "${download_dir}/llama.cpp.tar.gz" -C "${download_dir}/tree" \
    --strip-components=1 --wildcards '*/gguf-py/*'
  rm -f "${download_dir}/llama.cpp.tar.gz"
  if ! reference_complete "${download_dir}/tree"; then
    echo "error: extracted tree does not look like llama.cpp gguf-py" >&2
    exit 1
  fi
  rm -rf "${tree_dir}"
  mkdir -p "$(dirname "${tree_dir}")"
  mv "${download_dir}/tree" "${tree_dir}"
  rm -rf "${download_dir}"
  trap - EXIT
fi

# 2. Python with numpy. gguf-py's reader is numpy-based; if the default
#    interpreter cannot import it, try the nix shell the project docs use.
if ! "${python_cmd[@]}" -c 'import numpy' >/dev/null 2>&1; then
  if command -v nix >/dev/null 2>&1; then
    echo "numpy is not importable with '${python_cmd[0]}'; trying nix shell nixpkgs#python3Packages.numpy" >&2
    export XDG_CACHE_HOME="${XDG_CACHE_HOME:-${cache_dir}/nix-home}"
    mkdir -p "${XDG_CACHE_HOME}"
    python_cmd=(nix shell nixpkgs#python3Packages.numpy -c python3)
    if ! "${python_cmd[@]}" -c 'import numpy' >/dev/null 2>&1; then
      echo "error: could not obtain a Python interpreter with numpy." >&2
      echo "       Install numpy for '${HOARD_ORACLE_PYTHON:-python3}', or set HOARD_ORACLE_PYTHON." >&2
      exit 2
    fi
  else
    echo "error: '${python_cmd[0]}' cannot import numpy, and nix is not available." >&2
    echo "       Install numpy (pip install numpy) or set HOARD_ORACLE_PYTHON." >&2
    exit 2
  fi
fi

# 3. Build the local CLI used as the left-hand side of the comparison.
export CARGO_HOME="${CARGO_HOME:-${repo_root}/.cargo-home}"
echo "building hoard-gguf (CARGO_HOME=${CARGO_HOME})" >&2
cargo build --quiet --manifest-path "${repo_root}/Cargo.toml" -p hoard-gguf
target_dir="${CARGO_TARGET_DIR:-${repo_root}/target}"
hoard_bin="${target_dir}/debug/hoard-gguf"
if [[ ! -x "${hoard_bin}" ]]; then
  echo "error: expected hoard-gguf binary at ${hoard_bin}" >&2
  exit 1
fi

# 4. Files to compare: explicit arguments, or the minimum contracted set.
if [[ $# -gt 0 ]]; then
  files=("$@")
else
  real_fixture="${repo_root}/fixtures/real/stories260K.gguf"
  if [[ ! -f "${real_fixture}" ]]; then
    echo "real fixture is absent; fetching it with scripts/fetch-fixtures.sh" >&2
    (cd "${repo_root}" && bash scripts/fetch-fixtures.sh)
  fi
  files=(
    "${repo_root}/crates/hoard-gguf/tests/fixtures/synthetic-rich.gguf"
    "${repo_root}/crates/hoard-gguf/tests/fixtures/synthetic-minimal.gguf"
    "${real_fixture}"
  )
fi

missing=0
for file in "${files[@]}"; do
  if [[ ! -f "${file}" ]]; then
    echo "error: no such GGUF file: ${file}" >&2
    missing=1
  fi
done
if [[ "${missing}" -ne 0 ]]; then
  exit 2
fi

# 5. Compare. The comparator exits non-zero on the first file with any
#    difference (after printing every difference it found).
rc=0
"${python_cmd[@]}" "${repo_root}/scripts/gguf_oracle.py" \
  --llama-cpp-tree "${tree_dir}" \
  --hoard-gguf "${hoard_bin}" \
  --commit "${LLAMA_CPP_COMMIT}" \
  "${files[@]}" || rc=$?
exit "${rc}"
