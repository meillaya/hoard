# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project aims to follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.1.0

Initial release: offline, read-only GGUF inspection, manifests, and hashing.

### Added

- Three-crate Rust workspace (`hoard-gguf`, `hoard-core`, `hoard-cli`) on
  edition 2024, Apache-2.0, with `Cargo.lock` committed and a pinned 1.99.1
  toolchain. Workspace lints deny `unsafe_code`; crates deny missing docs.
- `hoard-gguf`: strict, zero-copy GGUF v2/v3 parser for little-endian files.
  Bounds-checked reads, checked arithmetic, a mirrored `ggml_type` table,
  metadata value types `u8`..`f64` including nested arrays, alignment and
  `data_start` computation, tensor bounds and block-geometry checks, and
  duplicate-name detection. Big-endian files are rejected with a dedicated
  diagnostic.
- `hoard-gguf`: stable kebab-case warning codes for unusual-but-readable files
  (unknown quantization ids, unaligned or overlapping tensors, non-zero
  padding, incomplete split metadata, metadata naming oddities, and more).
  Warnings never affect the exit code.
- `hoard-gguf`: frozen inspect report at `schema: 1` with file, model, split,
  metadata (sampled arrays), tensors, quant mix, memory, and warnings sections;
  committed `schemas/inspect-v1.schema.json` generated with `schemars`.
- `hoard-gguf` CLI: `inspect <path> [--json] [--tensors]` with a human report,
  the stable JSON report, and warnings on stderr; `schema` prints the inspect
  JSON Schema.
- `hoard-core`: manifest v1 model (`source` pinned to a Hugging Face repository
  and 40-hex commit sha, `files[]` with path, role, group, size, SHA-256,
  BLAKE3, and symmetric sibling relations). Parsing and validation are
  separate; validation rejects path traversal, unsorted or duplicate entries,
  malformed digests, broken sibling relations, and unknown fields.
- `hoard-core`: canonical JSON encoding (sorted keys at every level, two-space
  indent, LF, one trailing newline, sorted files and siblings, no timestamps)
  and the manifest artifact ID, `blake3:<hex>` over the canonical bytes.
- `hoard-core`: streaming SHA-256 and BLAKE3 file hashing in 1 MiB chunks, with
  typed lowercase-hex digest values and a committed manifest JSON Schema.
- `hoard` CLI: `hash <file> [--json]`, `manifest validate <path>`,
  `manifest fmt <path> [--check]`, `manifest hash <path>`, and
  `manifest schema`, sharing exit codes 0/1/2 and human output on stdout.
- Deterministic GGUF fixture builder and committed synthetic fixtures
  (`synthetic-minimal.gguf`, `synthetic-rich.gguf`) plus a byte-exact golden
  inspect report and a schema-drift test.
- `scripts/fetch-fixtures.sh`: downloads the real GGUF corpus
  (`stories260K.gguf`, `stories260K-be.gguf`) from `ggml-org/models` at a pinned
  revision and verifies each file's SHA-256. Tests that need the corpus skip
  when it is absent.
- Nix flake: devShell (cargo, rustc, rustfmt, clippy, git, curl, cacert, jq,
  shellcheck), package building both binaries, `hoard` and `hoard-gguf` apps,
  checks, nixfmt formatter, and a pinned `flake.lock`, for `x86_64-linux`,
  `aarch64-linux`, and `aarch64-darwin`.
- CI workflows: format/clippy/test/doc, Nix flake check and package smoke test,
  `cargo-deny` (advisories, bans, licenses, sources), and actionlint plus
  shellcheck for workflows and scripts.
- Documentation set: design and decisions, inspect JSON contract, manifest v1
  contract, CLI reference, limits and non-goals, security model, development
  guide, testing plan, and roadmap.

### Security

- All parsing is bounds-checked and capped; malformed input produces typed
  errors instead of panics or unbounded allocation. The single `unsafe`
  operation is a documented read-only `mmap`.
- Manifests are validated before they are hashed or used; there is no
  best-effort mode.
- 0.1.0 makes no network requests. The threat model for the planned transport
  (untrusted source responses, resume, cache poisoning) and the existing
  `mmap` truncation (`SIGBUS`) risk are documented.

### Known limitations

- No tensor decoding, no GGUF writer, no split/merge, no quantization tools.
- No fetching, content-addressed store, or cache; no Nix artifact helpers.
- Little-endian GGUF v2/v3 only; Linux x86_64 is the tested platform.
- No standalone validator, no fuzzing targets, and no differential harness
  against `llama.cpp` yet; see `docs/roadmap.md` for the plan.
