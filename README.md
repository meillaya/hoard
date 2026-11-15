# hoard

Inspect GGUF model files, hash them, and pin an artifact set to a canonical
manifest.

hoard answers two questions about a model download: what is inside this GGUF
file, and which exact bytes make up this artifact set. It is for Rust and Nix
users who fetch model weights from Hugging Face and want an offline check before
loading them. It runs offline and prints reports. The parser maps the file,
validates every structure up front, and returns a typed error for corrupted
input. The manifest layer records the source repository, the pinned commit, and
a digest per file.

## Try it

Build the workspace, fetch the test corpus, and inspect one file:

```sh
cargo build --workspace --release
./scripts/fetch-fixtures.sh
./target/release/hoard-gguf inspect fixtures/real/stories260K.gguf
```

```console
fixtures/real/stories260K.gguf
  size        1.13 MiB (1,185,376 bytes)
  format      GGUF v3 (little-endian)
  model       llama "llama"
  alignment   32 bytes
  data        starts at byte 14,176
  tensors     48, 1.12 MiB weights (98.8% of file)
  split       none

quantization
  F32            48 tensors    1.12 MiB  100.00%
```

Hash the same file and compute the artifact ID of the committed manifest:

```console
$ ./target/release/hoard hash fixtures/real/stories260K.gguf
fixtures/real/stories260K.gguf
  size    1185376
  sha256  270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d
  blake3  1b2a49c52e28947a42c0b93606e1cd4554dc97f046f37fb292fbaa3517acdcc0

$ ./target/release/hoard manifest hash fixtures/manifests/stories260k.json
blake3:56bbfeba5585a228ce8ac25e41d2fba1b16f80e837f93257b61d8fda11bb2421
```

`fixtures/real/` is gitignored, and `scripts/fetch-fixtures.sh` verifies each
file's SHA-256 after download. The synthetic fixtures used by the test suite are
committed, so `cargo test` runs without the corpus.

## Commands

| Command | Purpose |
| --- | --- |
| `hoard-gguf inspect <path> [--json] [--tensors]` | Validate a GGUF file and print the human report or the JSON report. |
| `hoard-gguf schema` | Print the inspect JSON Schema, version 1. |
| `hoard hash <file> [--json]` | Stream the SHA-256 and BLAKE3 digests of a file. |
| `hoard manifest validate <path>` | Check a manifest against every version 1 rule. |
| `hoard manifest fmt <path> [--check]` | Print canonical JSON; `--check` fails when the file is not canonical. |
| `hoard manifest hash <path>` | Print the manifest artifact ID as `blake3:<hex>`. |
| `hoard manifest schema` | Print the manifest JSON Schema, version 1. |

The flags, exit codes, and examples are in [usage](docs/usage.md).

## Status

| Phase | Scope | State |
| --- | --- | --- |
| P0 | Workspace, lints, toolchain, CI, Nix flake, fixtures, docs. | Done |
| P1 | GGUF parser, inspect report, manifest v1, hashing, CLI. | Done for 0.1.0 |

0.1.0 reads files and prints reports. The repository does not contain a
validator command, a GGUF writer, split or merge, tensor decoding, a fetcher, a
content-addressed store, or the Nix artifact commands. The phase plan and exit
criteria are in [roadmap](docs/roadmap.md).

## Architecture

```text
hoard-gguf   strict GGUF v2/v3 parser, inspect report, warning codes
             mmap view, one unsafe block behind a documented allow
hoard-core   manifest v1, canonical JSON, validation, streaming SHA-256/BLAKE3
hoard-cli    the `hoard` binary: hash and manifest commands
```

The format crate builds the `hoard-gguf` binary and does not depend on the
artifact layer. The artifact layer has no GGUF types and builds the `hoard`
binary. Both binaries parse arguments and format output; the logic lives in the
libraries and their tests.

The parser accepts GGUF version 2 and 3 files, little-endian, and rejects
everything else. Caps and warning codes are listed in
[limits and non-goals](docs/limitations.md) and the [inspect
contract](docs/inspect.md). The manifest rules, canonical encoding, and trust
model are in [manifest v1](docs/manifest.md).

## Develop

`devenv.nix` defines the development shell. Use it as the primary environment:

```sh
devenv shell
devenv test
```

`nix develop --impure` enters the same shell through the flake. A system or
rustup Rust 1.99.1 with plain `cargo` is the secondary path. See
[development](docs/development.md).

## Build with Nix

The flake provides a devShell, both binaries, apps, checks, and a formatter for
`x86_64-linux`, `aarch64-linux`, and `aarch64-darwin`:

```sh
nix develop --impure
nix build .#default
./result/bin/hoard-gguf inspect fixtures/real/stories260K.gguf
```

`flake.lock` pins nixpkgs. Intel macOS is absent from the system list because
nixpkgs 26.11 dropped it. Dependencies come from the committed `Cargo.lock` via
`cargoLock.lockFile`, so the build needs no vendoring hash.

## Requirements

- Primary development environment is `devenv shell`; it reads
  `rust-toolchain.toml` through rust-overlay and uses Rust 1.99.1.
- A system or rustup Rust 1.99.1 with plain `cargo` is the secondary path.
- Linux x86_64 is the tested platform. The flake evaluates `aarch64-linux` and
  `aarch64-darwin`; Windows is untested.
- Cargo fetches dependencies once. The binaries and tests make no network
  requests.

## Documentation

- [Design](docs/design.md): layers, boundaries, format subset, decisions.
- [Inspect report](docs/inspect.md): the frozen `schema: 1` JSON contract.
- [Manifest v1](docs/manifest.md): fields, canonical form, artifact ID.
- [CLI reference](docs/usage.md): commands, flags, exit codes.
- [Limits and non-goals](docs/limitations.md): what hoard does not do.
- [Security model](docs/security.md): threats, mitigations, residual risks.
- [Development](docs/development.md): toolchain, checks, Nix, CI, layout.
- [Testing](docs/testing.md): current suite and planned harnesses.
- [Roadmap](docs/roadmap.md): phases P0 to P4 and stopping points.
- [Fixtures](fixtures/README.md): synthetic and downloaded test corpus.

The crates are not published to crates.io yet. `hoard-gguf` is the first
publication target.

## Licence

Apache-2.0. See [LICENCE](LICENCE).
