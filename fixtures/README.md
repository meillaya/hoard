# Fixtures

hoard's tests use two kinds of GGUF data.

## Synthetic (committed)

`crates/hoard-gguf/tests/fixtures/*.gguf` are tiny deterministic files written
by the `hoard-gguf` crate's `fixture` module and checked in so tests need no
network and no external tooling. They are generated (and regenerated) with:

```sh
UPDATE_FIXTURES=1 cargo test -p hoard-gguf --features testing --test generate
```

| File | Purpose |
| --- | --- |
| `synthetic-minimal.gguf` | GGUF v3, no metadata, no tensors; smallest valid file. |
| `synthetic-rich.gguf` | GGUF v3 with every metadata value type, a nested array, split metadata, and F32/Q8_0/Q4_K tensors. Used for the golden `inspect --json` test and the truncation sweep. |

Do not edit these files by hand. The golden report lives next to them in
`crates/hoard-gguf/tests/golden/` and is regenerated the same way.

## Real (downloaded, not committed)

Real-model tests use two small files from the `ggml-org/models` repository at a
pinned commit:

| File | Bytes | SHA-256 |
| --- | --- | --- |
| `stories260K.gguf` | 1,185,376 | `270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d` |
| `stories260K-be.gguf` | 1,185,376 | `276d6f9d9050e79670a113492331dc52b3568690b03f5e633521cbed020bfc1c` |

Source: `ggml-org/models` at revision
`499bc8821c6b12b4e53c5bffcb21ec206f212d81`, paths
`tinyllamas/stories260K.gguf` and `tinyllamas/stories260K-be.gguf`.

`fixtures/real/` is gitignored. Fetch (or re-verify) the files with:

```sh
scripts/fetch-fixtures.sh            # download what is missing, verify what exists
scripts/fetch-fixtures.sh --force    # re-download unconditionally
```

The big-endian file is intentionally part of the corpus: hoard must reject it
with a clear diagnostic rather than misparse it.

Tests that need these files skip with a notice when they are absent, so
`cargo test` and `nix build` work in a clean checkout. CI runs the script before
the test suite.
