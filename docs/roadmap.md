# Roadmap

hoard is built in phases. Each phase is useful on its own; later phases do not
require rewriting earlier ones. The phase names are used throughout the
documentation, so "P2" means "transport", not "second half of the work".

| Phase | Scope | State in 0.1.0 |
| --- | --- | --- |
| P0 | Repository scaffold: workspace, toolchain, CI, flake, licence, fixture pipeline. | Done |
| P1 | Inspect MVP: GGUF v2/v3 parser, inspect report, manifest v1, hashing, CLI. | Done for 0.1.0; deferred items below |
| P2 | Artifact pipeline: fetch, verify, content-addressed store, resume. | Planned |
| P3 | Nix integration: fixed-output artifact derivations, offline builds. | Planned |
| P4 | Ecosystem: publication, signing/attestations, wider platform and format coverage. | Planned |

## P0 — Scaffold

- Three-crate workspace (`hoard-gguf`, `hoard-core`, `hoard-cli`), edition 2024,
  workspace lints (`unsafe_code` denied, `missing_docs` denied per crate).
- Pinned toolchain (`rust-toolchain.toml`), committed `Cargo.lock`, Apache-2.0.
- GitHub Actions: fmt/clippy/test/doc, Nix, cargo-deny, actionlint.
- Nix flake with devShell, package, apps, checks, formatter; pinned
  `flake.lock`.
- Fixture pipeline: committed synthetic GGUFs, downloaded real corpus with
  pinned SHA-256, golden inspect JSON.
- Documentation set: design, limits, security, development, testing, roadmap,
  inspect and manifest contracts.

## P1 — Inspect MVP (0.1.0)

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

Deferred from P1, tracked as follow-up work:

| Item | Why it is deferred |
| --- | --- |
| Standalone GGUF validator | The parser already validates structure on read; a separate "validate against the full spec" command needs the differential oracle to be meaningful. |
| Split and merge tools | Writing GGUF shards is a different feature set (writer, alignment rules, cross-part tensor accounting) and was not needed to prove the parser. |
| Quantization decode | Requires tensor-data decoding for every type; inspect deliberately reports metadata and sizes only. |
| Differential harness against `llama.cpp` | Needs a pinned oracle and a real corpus that is too large to commit; see [testing](testing.md). |
| `cargo-fuzz` targets | Needs a nightly toolchain and a corpus; planned with the transport work where untrusted input grows. |

## P2 — Artifact pipeline

Goal: make hoard actually move and verify model files, while preserving the
current guarantees.

- `hoard fetch`/`sync`: resolve a Hugging Face repository at a revision,
  download files, verify `sha256`, compute `blake3`, write the manifest.
- Content-addressed store: files named by BLAKE3, atomic writes, re-hash on
  read, no trust-on-first-use.
- Resume: partial downloads in temp files, safe restart after `SIGKILL`,
  cleanup of incomplete entries.
- Verify: offline re-verification of a manifest against a local directory or
  store, with machine-readable failures.
- Transport hardening: HTTPS only, bounded redirects, source host allow-list,
  size limits, pinned commit URLs.
- Threat model from [security](security.md) implemented and tested with a mock
  server.

Exit criteria: the fetch test matrix (success, wrong bytes, truncation,
redirect, resumekill, cache poison) is green, and no P1 behaviour changes.

## P3 — Nix integration

Goal: let Nix builds consume model artifacts as ordinary fixed-output
derivations.

- `fetchArtifact`-family helpers (or flake outputs) that take a pinned manifest
  and a file path and produce a Nix FOD.
- Decide how the manifest itself is pinned (flake input by artifact ID, or a
  download of a specific canonical manifest).
- Multi-file artifact sets: symlink farm or a single output directory.
- Offline round-trip test: fetch once, build twice with networking disabled,
  identical output hashes.
- Document the hash mapping between manifest digests and Nix store hashes.

Exit criteria: a sample derivation builds from a manifest on a clean machine
with one network fetch, and from the Nix store with none.

## P4 — Ecosystem and release

- crates.io publication. `hoard-gguf` is published first (the name is free);
  the short `hoard`/`hoard-cli` names are likely taken, so the CLI package name
  is resolved before its first release. Binary names stay `hoard` and
  `hoard-gguf`.
- Release automation: tag-driven builds, checksums, changelog checks.
- Signing/attestation: sign manifests or artifacts, or verify CI provenance;
  decide whether this belongs in the format or the transport layer.
- Platform matrix: CI for macOS/aarch64 Nix outputs; Windows support decision.
- Wider format coverage: quantization types added after the pinned table
  revision, GGUF v4 if it appears, and a writer path for split/merge/convert.

## Stopping points

These are deliberate scope cuts, not unfinished work:

- **No tensor decoding in 0.1.0.** Inspecting metadata and sizes is sufficient
  to build manifests and identify files; decoding every quantization type is a
  large, separate correctness problem.
- **No network in 0.1.0.** Transport is the next phase; shipping it half-done
  would weaken the security story.
- **Little-endian, v2/v3 only.** Reflects the format as used; converting
  big-endian files is not a goal.
- **No signatures.** Manifest v1 has no timestamp and no signature; identity is
  content hashing. Signing is a P4 decision, not a missing field.
- **No Intel macOS in the flake.** nixpkgs 26.11 dropped `x86_64-darwin`;
  carrying a stable-branch pin for one EOL platform would also pin the Rust
  toolchain. See [limits and non-goals](limitations.md).

## Risks

| Risk | Impact | Mitigation |
| --- | --- | --- |
| `llama.cpp` quantization table drifts from the mirrored copy. | Byte sizes for new type ids would be wrong or unknown. | Table is tested against a pinned commit; unknown ids are warnings, never guesses; revisit when bumping the pinned revision. |
| Hugging Face revision semantics change. | P2 fetch may resolve a different commit than it records. | Resolve to a commit sha first and fetch from the pinned sha URL; verify both digests. |
| Nix FOD hash mapping (BLAKE3 vs sha256) is unresolved. | P3 design could be blocked. | Manifest records both digests; the FOD sketch uses `sha256` and the artifact ID pins the manifest; decide before implementing the helper. |
| crates.io name collisions. | CLI crate publication blocked under the intended name. | Publish `hoard-gguf` first; rename the CLI package if needed, keep binary names. |
| Large-file memory-mapping faults (`SIGBUS`). | Process abort while inspecting a file being rewritten. | Documented limitation; hashing streams instead of mapping; never write model files. |
| Fuzzing and differential testing are not yet gates. | Parser regressions may not be caught before release. | Add both in P1 follow-up/P2; keep the synthetic fixture truncation sweep in the meantime. |
| macOS/aarch64 Nix outputs are not exercised locally. | Flake breakage could ship. | Evaluate all systems in CI via `nix flake show --all-systems`; build coverage later. |
