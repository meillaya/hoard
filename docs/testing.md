# Testing

The default test suite runs offline and needs no external services. It contains
deterministic unit tests, synthetic-fixture integration tests, golden JSON, a
mock Hugging Face server for the transport, and fixture tests that use the
downloaded real corpus when it is present. Two checks are opt-in: the live Xet
gate and the Nix round-trip.

## Running

Primary path, from the repository shell:

```sh
devenv test
```

That runs formatting, clippy, and the full test suite. The same suite with a
system or rustup Rust 1.99.1:

```sh
CARGO_HOME="$PWD/.cargo-home" cargo test --workspace --all-features
```

The same command runs in CI after `bash scripts/fetch-fixtures.sh`. Tests that
need `fixtures/real/` print a skip notice and pass when the files are absent,
so a clean checkout and the Nix sandbox both work without downloading models.

Per crate:

```sh
CARGO_HOME="$PWD/.cargo-home" cargo test -p hoard-gguf --all-features
CARGO_HOME="$PWD/.cargo-home" cargo test -p hoard-core
CARGO_HOME="$PWD/.cargo-home" cargo test -p hoard-cli
CARGO_HOME="$PWD/.cargo-home" cargo test -p hoard-hf
```

Feature-gated suites: `--all-features` enables the `testing` feature of
`hoard-gguf`, which exposes the deterministic fixture builder used by tests and
by fixture regeneration.

Inside `devenv shell`, the scripts `test`, `check`, `fixtures`, `xet-live`, and
`roundtrip` wrap the common commands.

### Live Xet gate

The Xet gate downloads from the real Hugging Face Hub and the Xet CAS network,
so it is ignored by default. Run it when you change the Hub client or a
transport:

```sh
cargo test -p hoard-hf --test live_xet -- --ignored --nocapture
```

The test fetches `tinyllamas/stories260K.gguf` from `ggml-org/models` at
revision `499bc8821c6b12b4e53c5bffcb21ec206f212d81`, forces the Xet transport,
and asserts the reported transport, the 1,185,376 byte count, the final file
size, and that the `.part` file was renamed away. It uses the pinned SHA-256
`270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d` and the
pinned Xet hash `7cfbf69e76aef0af92130af4e5c57a2a170423fb2a273b4f11a04bcc70088ba5`.
CI runs the gate on pushes to `main` and on manual dispatch, not on pull
requests.

### Nix path and round-trip check

`scripts/check-nix-roundtrip.sh` builds the consumer example in `examples/nix`,
copies the fixed-output derivations to a temporary local file cache, deletes the
symlink farm and the FODs, and rebuilds with networking through the temporary
cache only. It compares the two output paths and prints
`round-trip ok: <path>` when they match. A mismatch fails the script and prints
both paths.

```sh
bash scripts/check-nix-roundtrip.sh
```

The same script runs in the Nix CI job after `nix build .#default`. The build
also proves that the FOD store paths produced by `hoard store` match the ones
the Nix helper expects, because the output path embeds the file hash and name.

## Counts

The workspace has 341 tests and 3 doctests:

| Crate | Tests | Ignored by default |
| --- | --- | --- |
| `hoard-gguf` | 171 | 0 |
| `hoard-core` | 118 | 1 (`store_root_env_probe`) |
| `hoard-cli` | 7 | 0 |
| `hoard-hf` | 45 | 1 (`live_xet_downloads_stories260k`) |

`store_root_env_probe` is a child-process probe that the store-root tests spawn
with a controlled environment, so it is not meant to run on its own.

## Current tests

| Suite | Location | Covers |
| --- | --- | --- |
| Cursor and bounds | `crates/hoard-gguf/src/cursor.rs` | Reads at every offset, truncation, overflow. |
| Header and byte order | `crates/hoard-gguf/src/header.rs` | Magic, versions 2/3, and big-endian detection. |
| Metadata | `crates/hoard-gguf/src/metadata.rs` | Every value type, nested arrays, invalid UTF-8, invalid booleans, caps, key warnings, duplicates. |
| Tensors and quantization | `crates/hoard-gguf/src/tensor.rs`, `quant.rs` | Dimensions, checked element/byte arithmetic, block geometry, the full `ggml_type` table against the pinned `llama.cpp` constants, unknown ids, offset warnings, overlap detection. |
| File and split | `crates/hoard-gguf/src/file.rs`, `split.rs` | Alignment and `data_start` math, padding, whole-file parsing, split metadata states. |
| Inspect report | `crates/hoard-gguf/src/inspect.rs` | Report shape, ordering, samples, quant mix percentages, memory accounting, 64-bit fields. |
| Inspect schema | `crates/hoard-gguf/src/schema.rs` | Committed `schemas/inspect-v1.schema.json` equals the generated schema. |
| Manifest model | `crates/hoard-core/tests/manifest.rs` | Valid/invalid manifest matrix: paths, roles, groups, siblings, hashes, source rules, unknown fields. |
| Canonical JSON | `crates/hoard-core/tests/canonical.rs` | Byte-level canonical encoding, key ordering, trailing newline, sibling ordering. |
| Manifest hashing | `crates/hoard-core/tests/hash.rs` | Deterministic hash, equality with canonical bytes, streaming digest vectors. |
| Manifest fixture | `crates/hoard-core/tests/fixtures.rs` | Real fixture parses, validates, and hash-matches when the real file is present. |
| Manifest schema | `crates/hoard-core/tests/schema.rs` | Committed `schemas/manifest-v1.schema.json` equals the generated schema. |
| CAS store | `crates/hoard-core/tests/cas.rs` | Store layout, put and verify behaviour, corrupt and truncated blob rejection, materialize safety, concurrent puts, root resolution from `HOARD_STORE`, `XDG_CACHE_HOME`, `HOME`, and the relative fallback. |
| Manifest builder | `crates/hoard-core/tests/artifact.rs` | Role inference, split set detection and rejection rules, symmetric siblings, group placement, source/CAS disagreement, atomic canonical write, a build from a real store. |
| Hub client and transport | `crates/hoard-hf/tests/mock_hub.rs` | Resolve and list against a local `wiremock` server, statuses, redirects to allowed and foreign hosts, truncated bodies, transport selection, missing or malformed digests, the bearer token header, and retry on connection errors. |
| Live Xet gate | `crates/hoard-hf/tests/live_xet.rs` | Ignored by default. Real Xet download of the pinned file with transport, size, digest, and part-file assertions. |
| `hoard` CLI | `crates/hoard-cli/tests/cli.rs` | `hash`, `manifest validate/fmt/hash/schema`, exit codes, JSON output, real-file hashes when present. |
| `hoard-gguf` CLI | `crates/hoard-gguf/tests/cli_inspect.rs` | `inspect` human/`--json`/`--tensors` output, `schema`, errors, real-corpus numbers when present. |
| Robustness / property | `crates/hoard-gguf/tests/robustness.rs` | `proptest`: arbitrary bytes, GGUF-prefixed bytes, mutated/truncated/appended rich fixtures, and header mutations never panic the parse-to-report pipeline. |
| Malformed matrix | `crates/hoard-gguf/tests/malformed.rs` | Truncation at every byte of both committed fixtures, bad magic/version/byte order, unknown value types, invalid UTF-8 and booleans, huge counts and lengths, more than four dimensions, block mismatch, duplicate names, out-of-bounds tensor and data ranges, zero alignment, and every warning code. |
| Golden report | `crates/hoard-gguf/tests/golden.rs` | Byte-exact `inspect --json` for `synthetic-rich.gguf`, pretty-printing, and trailing newline. |
| Inspect JSON contract | `crates/hoard-gguf/tests/inspect_json.rs` | Section presence, metadata ordering and typed values, tensor ordering and file offsets, basis-point percentages, 64-bit round-trips, null bytes for unknown types, warning pairs. |
| Real corpus | `crates/hoard-gguf/tests/real_corpus.rs` | Shape and tensor-range checks for the little-endian file; big-endian rejection; skips when the corpus is absent. |
| Fixture generation | `crates/hoard-gguf/tests/generate.rs` | `UPDATE_FIXTURES=1` regeneration of the committed synthetic fixtures. |

The CLI tests cover the local commands. The fetch commands are covered at the
library boundary (`mock_hub`, `cas`, `artifact`) and by the live Xet gate; the
CLI glue has no integration test yet.

## Fixtures

| Tier | Committed | Contents |
| --- | --- | --- |
| Synthetic | yes | `synthetic-minimal.gguf` (32 bytes) and `synthetic-rich.gguf` (132,640 bytes, ~130 KiB; the F32 `[64,512]` tensor alone is 131,072 data bytes), generated by the fixture builder and deterministic. |
| Golden | yes | Byte-exact `inspect --json` output for `synthetic-rich.gguf`. |
| Real corpus | no (gitignored) | `stories260K.gguf` and `stories260K-be.gguf` from `ggml-org/models` at a pinned revision, verified by SHA-256. |
| Nix example | yes | `examples/nix/models/stories15M-q8_0.json`, a three-part split manifest used by the consumer example and the round-trip check. |

The real corpus is fetched by `scripts/fetch-fixtures.sh`; see
[fixtures](../fixtures/README.md) for provenance and regeneration commands.
The big-endian file is a negative test: hoard must reject it with the
`big-endian` diagnostic.

## Planned: P1 differential harness

Structural unit tests confirm hoard is self-consistent. They cannot show that
it agrees with `llama.cpp`. P1 adds a differential harness:

- Pin one `llama.cpp` commit for the oracle (the quantization table currently
  mirrors `de32b268c4ad55d45f3b7b16c029f4e077f4ba74`, `gguf-py/gguf/constants.py`)
  and vendor or pin its reader; the table and the oracle must never drift
  independently.
- For every file in a real corpus, run `hoard-gguf inspect --json` and the
  oracle, then compare: file version, alignment, `data_start`, metadata count,
  tensor names, shapes, types, offsets, element counts, and computed byte
  sizes.
- Corpus: tiny Llama models, at least one split model (multiple `split.*`
  parts), one `mmproj` file, non-`llama` architectures, and every quantization
  type the table knows, plus a file with an unknown type id.
- Keep the corpus out of the repository and skip the suite when the files are
  absent, exactly like the current real-corpus tests.

## Planned: fuzzing

`cargo-fuzz` targets over `Gguf::parse` and `inspect`:

- `fuzz_parse`: arbitrary bytes must return `Ok`/`Err` and must never panic,
  hang, or allocate without bound.
- `fuzz_inspect`: parse a bounded input, then serialize the report; the report
  must always serialize to valid JSON.
- Seed the corpora with the committed synthetic fixtures and the downloaded
  real files. Use the parser caps as the oracle for memory: inputs that exceed
  a cap must fail before allocation.
- Run locally during parser changes and nightly in CI; do not gate pull
  requests on a fuzzing time budget.

## Boundaries

- Only `x86_64-linux` is exercised locally. CI runs the same commands on
  `ubuntu-24.04`; macOS and aarch64 Nix outputs are evaluated but not built
  locally.
- The live Xet gate and the Nix round-trip need network access on a cold store,
  so local runs may skip or fail without connectivity. The default suite does
  not.
- There are no benchmarks in 0.1.0. Inspecting, hashing, and fetching are
  linear in file size by design; a benchmark suite is a later addition.
- The suite does not cover Windows, 32-bit hosts, or big-endian hosts.
- The fetch path is tested against a mock server and one public repository. A
  private repository, a generic source, and a chunk-level store are not covered.
