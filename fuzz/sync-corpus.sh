#!/usr/bin/env bash
#
# Regenerate the committed fuzz seed corpus.
#
# Seeds are copied from the committed synthetic fixtures plus a handful of tiny
# malformed samples derived from them. Run this after the synthetic fixtures
# change so the corpus does not drift:
#
#   bash fuzz/sync-corpus.sh
#
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixtures_dir="${repo_root}/crates/hoard-gguf/tests/fixtures"
fuzz_dir="${repo_root}/fuzz"
minimal="${fixtures_dir}/synthetic-minimal.gguf"
rich="${fixtures_dir}/synthetic-rich.gguf"

for fixture in "${minimal}" "${rich}"; do
  if [[ ! -f "${fixture}" ]]; then
    echo "error: missing synthetic fixture ${fixture}" >&2
    echo "       regenerate with UPDATE_FIXTURES=1 cargo test -p hoard-gguf --features testing --test generate" >&2
    exit 1
  fi
done

for target in parse_gguf manifest_or_inspect_json; do
  corpus="${fuzz_dir}/corpus/${target}"
  rm -rf "${corpus}"
  mkdir -p "${corpus}"
  # Valid files: these give the fuzzer structurally interesting starting points.
  cp "${minimal}" "${corpus}/synthetic-minimal.gguf"
  cp "${rich}" "${corpus}/synthetic-rich.gguf"
done

# Small malformed samples, generated deterministically from the 32-byte minimal
# fixture so they are cheap to commit and to mutate.
python3 - "${minimal}" "${fuzz_dir}/corpus/parse_gguf" "${fuzz_dir}/corpus/manifest_or_inspect_json" <<'PY'
import pathlib
import struct
import sys

minimal = pathlib.Path(sys.argv[1]).read_bytes()
assert len(minimal) == 32, f"unexpected minimal fixture length {len(minimal)}"
assert minimal[:4] == b"GGUF"

seeds = {
    "empty": b"",
    "bad-magic": b"GGML" + minimal[4:],
    "bad-version": minimal[:4] + struct.pack("<I", 1) + minimal[8:],
    # A big-endian writer stores version 3 as 00 00 00 03; read as
    # little-endian that is the 0x03000000 hint hoard must reject.
    "big-endian-version": minimal[:4] + struct.pack(">I", 3) + minimal[8:],
    "truncated-header": minimal[:16],
    "header-only": minimal[:24],
    "non-zero-padding": minimal[:24] + b"\xff" + minimal[25:],
}

for corpus in sys.argv[2:]:
    corpus_dir = pathlib.Path(corpus)
    for name, data in seeds.items():
        (corpus_dir / f"{name}.gguf").write_bytes(data)
    print(f"{corpus_dir}: {len(seeds) + 2} seeds")
PY
