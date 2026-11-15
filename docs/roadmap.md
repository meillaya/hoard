# Roadmap

hoard is built in phases. Each phase is useful on its own; later phases do not
require rewriting earlier ones. The phase names are used throughout the
documentation, so "P2" means "transport" rather than "second half of the work".

| Phase | Scope | State in 0.1.0 |
| --- | --- | --- |
| P0 | Repository scaffold: workspace, toolchain, CI, flake, licence, fixture pipeline. | Done |
| P1 | Inspect MVP: GGUF v2/v3 parser, inspect report, manifest v1, hashing, CLI. | Done for 0.1.0; deferred items below |
| P2 | Artifact pipeline: fetch, verify, content-addressed store, resume. | Partially shipped: fetch, verify, store, push, CAS, resume. |
| P3 | Nix integration: fixed-output artifact derivations, offline builds. | Partially shipped: per-file flat FODs, `fetchArtifactSet`, round-trip check. |
| P4 | Ecosystem: publication, signing/attestations, wider platform and format coverage. | Planned |

## P0: Scaffold

- Four-crate workspace (`hoard-gguf`, `hoard-core`, `hoard-hf`, `hoard-cli`),
  edition 2024, workspace lints (`unsafe_code` denied, `missing_docs` denied per
  crate).
- Pinned toolchain (`rust-toolchain.toml`), committed `Cargo.lock`, Apache-2.0.
- `devenv.nix` primary development shell; the flake devShell delegates to it;
  CI runs devenv as the primary job and plain cargo as a secondary job.
- GitHub Actions: devenv test, fmt/clippy/test/doc, live Xet gate, Nix
  round-trip, cargo-deny, actionlint.
- Nix flake with devShell, package, apps, checks, lib helpers, formatter;
  pinned `flake.lock` and `devenv.lock`.
- Fixture pipeline: committed synthetic GGUFs, downloaded real corpus with
  pinned SHA-256, golden inspect JSON, and the example split manifest.
- Documentation set: design, limits, security, development, testing, roadmap,
  usage, fetch, inspect and manifest contracts.

## P1: Inspect MVP (0.1.0)

Shipped:

- `hoard-gguf inspect` and `hoard-gguf schema`.
- Strict GGUF v2/v3 little-endian parsing with caps, typed errors, stable
  warning codes, and zero-copy metadata/tensor views.
- Frozen `inspect` JSON report (`schema: 1`) and committed JSON Schema.
- Manifest v1 types, canonical JSON, validation, SHA-256/BLAKE3 streaming
  hashes, committed JSON Schema.
- `hoard hash`, `hoard manifest validate|fmt|hash|schema`.
- Real and synthetic fixtures, golden-report tests, schema-drift tests, and the
  documented limits.
- Differential oracle against `llama.cpp` commit
  `de32b268c4ad55d45f3b7b16c029f4e077f4ba74`: 3 of 3 reference files match.
- `cargo-fuzz` targets for parsing and inspect JSON, with a nine-seed corpus
  per target; short campaigns found no crashes.

Deferred from P1, tracked as follow-up work:

| Item | Why it is deferred |
| --- | --- |
| Standalone GGUF validator | The parser already validates structure on read; a separate "validate against the full spec" command needs broader corpus and reviewer input. |
| Split and merge tools | Writing GGUF shards is a different feature set (writer, alignment rules, cross-part tensor accounting) and was not needed to prove the parser. |
| Quantization decode | Requires tensor-data decoding for every type; inspect deliberately reports metadata and sizes only. |

## P2: Artifact pipeline

Goal: hoard fetches, verifies, and stores model files and keeps the guarantees
P1 established.

Shipped in 0.1.0:

- `hoard fetch` with a repository spec or an existing manifest, `--file`
  selection, `--group`, `--role`, `--repo-type`, `--manifest-out`, `--out`,
  `--transport auto|xet|http`, and `--offline`.
- `hoard-hf`: Hub revision resolution and recursive listing, Xet and HTTP
  transports, Range resume, retries with backoff, an 8-hop redirect policy with
  a host allow-list, and token handling from the environment.
- `hoard-core::cas`: whole-file content-addressed store keyed by BLAKE3, atomic
  writes, re-hash on read, and verified materialization.
- `hoard-core::artifact`: selection, role inference, split sibling detection,
  and a canonical manifest builder that joins the source listing with CAS
  entries.
- `hoard verify` against a directory or the CAS, `hoard store` for flat Nix
  store paths, and `hoard push` through `nix copy`.
- Transport-hardening tests against a local mock server, plus the live Xet gate.

Remaining for P2:

- Generic sources: a source abstraction for non-Hugging Face repositories, local
  directories, and mirrors that keeps the digest policy.
- Chunk-level dedupe decision: file-level CAS or content-defined chunking. Xet
  already deduplicates transfer; local dedupe is the open question.
- Plan and prune: a dry-run and a store GC that cannot delete a blob a manifest
  still references.
- Private repository coverage: the token path exists, but the suite only
  exercises public repositories.

## P3: Nix integration

Goal: Nix builds consume model artifacts as ordinary fixed-output derivations.

Shipped in 0.1.0:

- `lib.fetchArtifact` and `lib.fetchArtifactSet` in `flake.nix`.
- Flat fixed-output derivations, one per manifest file, with
  `outputHashMode = "flat"` and the manifest `sha256`. Nix store paths are
  `hoard-<first 12 characters of sha256>-<basename>`.
- A symlink farm over the per-file FODs for a whole artifact set.
- `examples/nix`, a consumer flake that takes `github:meillaya/hoard` and builds
  `examples/nix/models/stories15M-q8_0.json`.
- `hoard store`, which reproduces the FOD path through
  `nix-store --add-fixed sha256`, and `hoard push`, which delegates upload and
  signing to `nix copy`.
- `scripts/check-nix-roundtrip.sh`, the offline path-equality check, wired into
  the Nix CI job.

Remaining for P3:

- Manifest pinning: pin the manifest itself as a flake input by artifact ID
  instead of passing a path.
- Upstream the helper pattern as a nixpkgs PR or a published flake template.
- Decide whether Nix should learn BLAKE3 directly instead of mapping to the
  manifest `sha256`.

## P4: Ecosystem and release

- crates.io publication. `hoard-gguf` is published first (the name is free);
  the short `hoard`/`hoard-cli` names are likely taken, so the CLI package name
  is resolved before its first release. Binary names stay `hoard` and
  `hoard-gguf`.
- Release automation: tag-driven builds, checksums, changelog checks.
- Parser assurance: run the committed `llama.cpp` differential harness and the
  `cargo-fuzz` targets on the release commit before publishing.
- Signing/attestation: sign manifests or artifacts, or verify CI provenance;
  decide whether this belongs in the format, the transport, or the cache.
- Platform matrix: CI for macOS/aarch64 Nix outputs; Windows support decision.
- Wider format coverage: quantization types added after the pinned table
  revision, GGUF v4 if it appears, and a writer path for split/merge/convert.

## Stopping points

These scope cuts are deliberate:

- No tensor decoding in 0.1.0. Inspecting metadata and sizes is sufficient to
  build manifests and identify files; decoding every quantization type is a
  large, separate correctness problem.
- One network command. `hoard fetch` is the network path; every other command
  works offline. Shipping a generic transport before the source abstraction
  would weaken the security story.
- Little-endian, v2/v3 only. hoard follows the format as it is written in
  practice; converting big-endian files is not a goal.
- No signatures in hoard. Manifest v1 has no timestamp and no signature;
  identity is content hashing. Signing is a P4 decision, and cache signing is
  delegated to Nix.
- No Intel macOS in the flake. nixpkgs 26.11 dropped `x86_64-darwin`;
  carrying a stable-branch pin for one EOL platform would also pin the Rust
  toolchain. See [limits and non-goals](limitations.md).

## Risks

| Risk | Impact | Mitigation |
| --- | --- | --- |
| `llama.cpp` quantization table drifts from the mirrored copy. | Byte sizes for new type ids would be wrong or unknown. | Table is tested against a pinned commit; unknown ids are warnings, never guesses; revisit when bumping the pinned revision. |
| Hugging Face revision semantics change. | Fetch may resolve a different commit than it records. | Resolve to a commit sha first and fetch from the pinned sha; verify both digests. Shipped in 0.1.0. |
| `hf-xet` and its native dependency tree change or fail to build. | The fetch path and the Nix package break. | The tree is locked in `Cargo.lock`, covered by cargo-deny, and built in CI; the Nix package supplies `aws-lc` from nixpkgs. |
| The Nix helper pattern changes upstream. | The local `fetchArtifact` helpers could diverge from Nix conventions. | Per-file flat FODs map to supported sha256 hashes; send the pattern upstream and keep the round-trip check as the gate. |
| The CAS grows without bound. | Disk exhaustion on long-lived machines. | The store is whole-file and re-hashed on read, so deleting it is always safe; a plan/prune path is on the P2 list. |
| Cache signing and credentials are mishandled. | Pushed paths cannot be verified elsewhere, or keys leak. | Hoard holds no keys; `nix copy` and the cache own signing, and the docs say so. |
| Private repositories are untested. | The token path may fail for gated files. | Anonymous public coverage in CI plus the live gate; add a private-repository check before claiming support. |
| crates.io name collisions. | CLI crate publication blocked under the intended name. | Publish `hoard-gguf` first; rename the CLI package if needed, keep binary names. |
| Large-file memory-mapping faults (`SIGBUS`). | Process abort while inspecting a file being rewritten. | Documented limitation; hashing streams instead of mapping; never write model files. |
| Fuzzing and differential testing are not yet gates. | Parser regressions may not be caught before release. | Add both in P1 follow-up/P2; keep the synthetic fixture truncation sweep in the meantime. |
| macOS/aarch64 Nix outputs are not exercised locally. | Flake breakage could ship. | Evaluate all systems in CI via `nix flake show --all-systems`; build coverage later. |
