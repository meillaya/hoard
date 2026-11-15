# hoard

Inspect GGUF model files, fetch and verify Hugging Face artifacts, pin them in a
canonical manifest, and consume the pinned bytes as Nix fixed-output
derivations.

hoard answers three questions about a model download: what is inside this GGUF
file, which exact bytes make up this artifact set, and do the bytes on disk
match the pinned manifest. It is for Rust and Nix users who fetch model weights
from Hugging Face and want a reproducible artifact in a build. The parser maps
the file, validates every structure up front, and returns a typed error for
corrupted input. The manifest layer records the source repository, the pinned
commit, and a digest per file. The fetch layer resolves a revision to a commit,
downloads through Xet or HTTP, verifies both digests, and stores bytes by BLAKE3
in a content-addressed store.

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

Fetch a file from the Hub, verify it, and write a manifest:

```console
$ ./target/release/hoard fetch hf:ggml-org/models@499bc8821c6b12b4e53c5bffcb21ec206f212d81 \
    --file tinyllamas/stories260K.gguf --manifest-out stories260K.json --out artifacts/
fetched tinyllamas/stories260K.gguf (1185376 bytes, Xet, 0.76s)
wrote stories260K.json (blake3:56bbfeba5585a228ce8ac25e41d2fba1b16f80e837f93257b61d8fda11bb2421)

$ ./target/release/hoard verify --manifest stories260K.json --dir artifacts/
verified 1 file(s)
```

The fetch workflow, offline mode, and the Nix consumer are in
[fetch and verify artifacts](docs/fetch.md).

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
| `hoard fetch <spec> --file <path> --manifest-out <file>` | Resolve a Hub revision, download and verify files into the CAS, write a manifest. |
| `hoard fetch --manifest <file> [--offline]` | Fill the CAS from an existing manifest, or check it offline. |
| `hoard verify --manifest <file> [--dir <dir> | --store]` | Check a manifest against a directory or the CAS. |
| `hoard store --manifest <file>` | Add the manifest files to the Nix store as flat fixed-output paths. |
| `hoard push --manifest <file> --to <store-uri>` | Copy the flat store paths to a Nix binary cache. |

The flags, exit codes, and examples are in [usage](docs/usage.md). The fetch
workflow is in [fetch and verify artifacts](docs/fetch.md).

## Status

| Phase | Scope | State |
| --- | --- | --- |
| P0 | Workspace, lints, toolchain, CI, Nix flake, fixtures, docs. | Done |
| P1 | GGUF parser, inspect report, manifest v1, hashing, CLI. | Done for 0.1.0 |
| P2 | Verified fetch, content-addressed store, manifest builder. | Partially shipped: fetch, verify, store, push, CAS, resume. |
| P3 | Nix integration: fixed-output artifact derivations. | Partially shipped: `fetchArtifact`, `fetchArtifactSet`, flat FODs, round-trip check. |
| P4 | Publication, signing, wider format and platform coverage. | Planned |

0.1.0 fetches Hugging Face LFS/Xet files, verifies both digests, stores bytes by
BLAKE3, and writes canonical manifests. It does not write GGUF files, split or
merge models, decode tensors, sign manifests, or fetch from a source other than
Hugging Face. The phase plan and exit criteria are in
[roadmap](docs/roadmap.md).

## Architecture

```text
hoard-gguf   strict GGUF v2/v3 parser, inspect report, warning codes
             mmap view, one unsafe block behind a documented allow
hoard-core   manifest v1, canonical JSON, validation, streaming hashes,
             content-addressed store, manifest builder
hoard-hf     Hugging Face resolution, Xet and HTTP transport, retry and
             redirect policy
hoard-artifact  the `hoard` binary: hash, manifest, fetch, verify, store, push
```

The format crate builds the `hoard-gguf` binary and does not depend on the
artifact layer. The artifact layer has no GGUF types. The transport crate
depends on neither the format layer nor the artifact layer. The CLI composes
all three, and both binaries parse arguments and format output; the logic lives
in the libraries and their tests.

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

The flake also exports `lib.fetchArtifact` and `lib.fetchArtifactSet`, which
turn manifest entries into flat fixed-output derivations and a symlink farm. The
consumer example is in `examples/nix/flake.nix`, and its manifest is
`examples/nix/models/stories15M-q8_0.json`. See
[fetch and verify artifacts](docs/fetch.md).

## Requirements

- Primary development environment is `devenv shell`; it reads
  `rust-toolchain.toml` through rust-overlay and uses Rust 1.99.1.
- A system or rustup Rust 1.99.1 with plain `cargo` is the secondary path.
- Linux x86_64 is the tested platform. The flake evaluates `aarch64-linux` and
  `aarch64-darwin`; Windows is untested.
- The first `hoard fetch` needs network access to the endpoint. Everything else
  runs offline, the default test suite uses a local mock server, and the live
  Xet gate runs only when you ask for it.
- Cargo fetches dependencies once.

## Documentation

- [Design](docs/design.md): layers, boundaries, format subset, decisions.
- [Inspect report](docs/inspect.md): the frozen `schema: 1` JSON contract.
- [Manifest v1](docs/manifest.md): fields, canonical form, artifact ID.
- [CLI reference](docs/usage.md): commands, flags, exit codes, environment.
- [Fetch and verify artifacts](docs/fetch.md): fetch workflow, CAS, Nix consumer.
- [Nix packaging](packaging/nixpkgs/README.md): draft nixpkgs expression and review notes.
- [Limits and non-goals](docs/limitations.md): what hoard does not do.
- [Security model](docs/security.md): threats, mitigations, residual risks.
- [Development](docs/development.md): toolchain, checks, Nix, CI, layout.
- [Testing](docs/testing.md): suite, live network gate, Nix round-trip.
- [Roadmap](docs/roadmap.md): phases P0 to P4 and stopping points.
- [Fixtures](fixtures/README.md): synthetic and downloaded test corpus.

The 0.1.0 crates are on crates.io: `hoard-gguf`, `hoard-core`, `hoard-hf`, and
`hoard-artifact`. The package `hoard-artifact` installs the `hoard` binary; the
`hoard` and `hoard-cli` names are taken by other projects.

```sh
cargo install hoard-artifact --locked
cargo install hoard-gguf --locked
```

## Licence

Apache-2.0. See [LICENCE](LICENCE).
