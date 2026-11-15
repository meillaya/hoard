# hoard

Verifiable, content-addressed artifacts for large model files: strict GGUF
inspection, deterministic manifests, and streaming content hashes.

Large model files arrive as bytes behind a URL. hoard gives those bytes an
identity you can check: it inspects a GGUF file without decoding its tensors,
hashes files with SHA-256 and BLAKE3, and pins an artifact set in a canonical
manifest whose BLAKE3 hash is the artifact ID. 0.1.0 is offline and read-only;
it does not download, convert, split, merge, or serve models.

## Status

| Phase | Scope | State |
| --- | --- | --- |
| P0 | Scaffold: workspace, lints, toolchain, CI, Nix flake, fixtures, docs. | Done |
| P1 | Inspect MVP: GGUF v2/v3 parser, inspect JSON, manifest v1, hashing, CLI. | Done for 0.1.0 |

Deferred beyond 0.1.0: standalone GGUF validator, split/merge, quantization
decode, fuzzing (`cargo-fuzz` targets), the differential harness against
`llama.cpp`, transport (fetch/verify/CAS), and Nix artifact integration. The
phase plan and exit criteria are in [roadmap](docs/roadmap.md).

## Quickstart

```sh
cargo build --workspace --release

# What is this file?
./target/release/hoard-gguf inspect model.gguf

# Machine-readable report.
./target/release/hoard-gguf inspect model.gguf --json | jq '.tensors.count'

# Which quantization types does it use?
./target/release/hoard-gguf inspect model.gguf --json \
  | jq -r '.quant_mix[] | "\(.type)\t\(.percent)%"'

# Source digest (matches the Hugging Face LFS oid) and content address.
./target/release/hoard hash model.gguf

# Manifest gates and artifact ID.
./target/release/hoard manifest validate manifest.json
./target/release/hoard manifest fmt manifest.json --check
./target/release/hoard manifest hash manifest.json
./target/release/hoard manifest schema
```

Real output from the committed corpus:

```console
$ hoard-gguf inspect fixtures/real/stories260K.gguf
fixtures/real/stories260K.gguf
  size        1.13 MiB (1,185,376 bytes)
  format      GGUF v3 (little-endian)
  model       llama "llama"
  alignment   32 bytes
  data        starts at byte 14,176
  tensors     48, 1.12 MiB weights (98.8% of file)
  split       none
...

$ hoard hash fixtures/real/stories260K.gguf
fixtures/real/stories260K.gguf
  size    1185376
  sha256  270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d
  blake3  1b2a49c52e28947a42c0b93606e1cd4554dc97f046f37fb292fbaa3517acdcc0

$ hoard manifest hash fixtures/manifests/stories260k.json
blake3:56bbfeba5585a228ce8ac25e41d2fba1b16f80e837f93257b61d8fda11bb2421
```

The full command reference, flags, exit codes, and examples are in
[usage](docs/usage.md).

## Commands

| Command | Purpose |
| --- | --- |
| `hoard-gguf inspect <path> [--json] [--tensors]` | Validate a GGUF file and print the report. |
| `hoard-gguf schema` | Print the inspect JSON Schema (v1). |
| `hoard hash <file> [--json]` | Stream SHA-256 and BLAKE3 digests. |
| `hoard manifest validate <path>` | Validate a manifest against every v1 rule. |
| `hoard manifest fmt <path> [--check]` | Print canonical JSON; `--check` is a CI gate. |
| `hoard manifest hash <path>` | Print `blake3:<hex>`, the manifest artifact ID. |
| `hoard manifest schema` | Print the manifest JSON Schema (v1). |

## Architecture

```text
hoard-gguf   strict GGUF v2/v3 parser, inspect report, warning codes
             (mmap + zero-copy view, one audited unsafe)
hoard-core   manifest v1, canonical JSON, validation, streaming SHA-256/BLAKE3
hoard-cli    the `hoard` binary: hash and manifest commands

hoard-gguf   ->  `hoard-gguf` binary
hoard-core   ->  `hoard-cli`  ->  `hoard` binary
```

The format layer does not depend on the artifact layer, and the artifact layer
has no GGUF types; both binaries are thin wrappers over tested libraries.
Formats and invariants are documented in [design](docs/design.md), the frozen
JSON contract in [inspect](docs/inspect.md), and the manifest format in
[manifest](docs/manifest.md).

## Build with Nix

The flake provides a devShell, the package (both binaries), apps, checks, and a
formatter for `x86_64-linux`, `aarch64-linux`, and `aarch64-darwin`:

```sh
nix develop
nix build .#default
./result/bin/hoard --version
./result/bin/hoard-gguf --version
```

The flake pins nixpkgs in `flake.lock`. `x86_64-darwin` is not in the system
list because nixpkgs 26.11 dropped Intel macOS; see
[limits and non-goals](docs/limitations.md). Dependencies come from the
committed `Cargo.lock` (`cargoLock.lockFile`), so `nix build` needs no vendoring
hash; see [development](docs/development.md).

## Requirements

- Rust 1.99.1 (selected by `rust-toolchain.toml`), edition 2024.
- Linux x86_64 is the tested platform. The flake also evaluates
  `aarch64-linux` and `aarch64-darwin`; no Windows support is promised.
- No network is needed after dependencies are fetched.

## Documentation

- [Design](docs/design.md) — layers, boundaries, format subset, decisions.
- [Inspect report](docs/inspect.md) — frozen `schema: 1` JSON contract.
- [Manifest v1](docs/manifest.md) — fields, canonical form, artifact ID.
- [CLI reference](docs/usage.md) — every command, flag, and exit code.
- [Limits and non-goals](docs/limitations.md) — what hoard does not do.
- [Security model](docs/security.md) — threats, mitigations, residual risks.
- [Development](docs/development.md) — toolchain, checks, Nix, CI, layout.
- [Testing](docs/testing.md) — current suite and planned harnesses.
- [Roadmap](docs/roadmap.md) — phases P0–P4 and stopping points.
- [Fixtures](fixtures/README.md) — synthetic and downloaded test corpus.

## Licence

Apache-2.0. See [LICENCE](LICENCE).
