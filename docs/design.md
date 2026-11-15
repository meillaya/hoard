# Design

hoard is a set of tools for moving large model files without trusting the
transport. It starts from the GGUF container format used by `llama.cpp`:
`hoard-gguf` answers "what is in this file", and `hoard` answers "which bytes
make up this artifact set, and what is their identity".

The long-term shape is five layers:

| Layer | Shipped in 0.1.0 | Responsibility |
| --- | --- | --- |
| Format | yes | `hoard-gguf`: strict GGUF parsing, inspect report, warning codes. |
| Artifact | yes (library + CLI) | `hoard-core`: manifest v1, canonical JSON, validation, streaming hashes, content-addressed store, manifest builder. |
| Tooling | yes | `hoard` and `hoard-gguf` binaries: commands, JSON output, exit codes. |
| Transport | partial | `hoard-hf` and `hoard fetch`: resolve a Hub commit, download over Xet or HTTP, verify, populate the CAS. |
| Nix integration | partial | `lib.fetchArtifact`, `lib.fetchArtifactSet`: flat fixed-output derivations and a symlink farm over a pinned manifest. |

0.1.0 splits along the network. `hoard-gguf`, `hoard hash`, and the manifest
commands stay local and offline. `hoard fetch` resolves a Hugging Face revision,
downloads through Xet or plain HTTP, verifies the source SHA-256 and the computed
BLAKE3, and writes the bytes to a content-addressed store before it writes a
manifest. The Nix helpers consume a pinned manifest and do not import hoard code
into the build.

## Crates and boundaries

| Crate | Binary | Depends on | Boundary |
| --- | --- | --- | --- |
| `hoard-gguf` | `hoard-gguf` | `memmap2`, `serde`, `serde_json`, `schemars`, `thiserror`, optional `clap`/`anyhow` | GGUF only. Does not know about manifests, hashes, the network, or other artifacts. |
| `hoard-core` | none | `blake3`, `sha2`, `hex`, `serde`, `serde_json`, `schemars`, `thiserror`, `tempfile` | Manifest, hashing, CAS, and manifest builder. Does not link `hoard-gguf` or `hoard-hf`; the artifact layer has no network types. |
| `hoard-hf` | none | `hf-xet`, `reqwest`, `serde`, `serde_json`, `sha2`, `tempfile`, `thiserror`, `url` | Hugging Face resolution and transport only. Does not link `hoard-core`; verification against a manifest belongs to the caller. |
| `hoard-cli` | `hoard` | `hoard-core`, `hoard-hf`, `anyhow`, `clap`, `serde_json`, `tempfile` | Command surface. Does not link `hoard-gguf`; the two binaries stay independent. |

The dependency direction is deliberate. The format layer works on its own, the
artifact layer has no GGUF or network types, and the transport layer knows
nothing about manifests or CAS paths. The CLI composes the artifact and
transport layers, so a new source can slot in behind the transport boundary
without changing the manifest identity.

Both binaries are thin: argument parsing, formatting, and error context. The
libraries hold the logic, and their unit tests exercise it.

## Zero-copy and unsafe

`hoard-gguf` maps the file read-only and parses a borrowed view:

```text
GgufFile (mmap) -> bytes() -> Gguf<'a> (borrows &'a [u8])
                               ├── metadata keys/string values borrowed
                               ├── tensor names borrowed
                               └── entry lists allocated once
```

- `GgufFile::open` memory-maps the file and rejects empty files.
- `Gguf::parse` validates the header, metadata, and tensor infos, computes the
  aligned data start, and checks every tensor range against the file length
  before it returns. Accessors are then infallible; only `split()` can still
  fail, because split metadata validation is deferred.
- Metadata keys, string values, and tensor names are slices of the mapped file.
  Parsing allocates memory only for the metadata entry list, array values, and
  tensor infos. Inspecting a multi-gigabyte file copies no bytes.
- The workspace has exactly one `unsafe` operation: `memmap2::Mmap::map` in
  `crates/hoard-gguf/src/file.rs`, under a single documented
  `#[allow(unsafe_code)]`. The workspace lint denies `unsafe_code` everywhere
  else. The `mmap` hazard (exiting process, file truncated by another process)
  is recorded in [security](security.md).

## Format subset

`hoard-gguf` checks structure strictly and accepts files that are unusual but
readable. It parses GGUF version 2 and 3, little-endian only:

| Rule | Behaviour |
| --- | --- |
| Magic | `GGUF`; anything else is `bad-magic`. |
| Version | 2 or 3; otherwise `unsupported-version`. A byte-swapped version word is reported as `big-endian` with a dedicated message. |
| Byte order | Little-endian only. Big-endian files are rejected, not byte-swapped. |
| Strings | Length-prefixed UTF-8; invalid UTF-8 is an error. |
| Metadata value types | Ids `0..=12` (`u8`..`f64`); unknown ids are an error. Arrays may nest. |
| Booleans | Payload must be `0` or `1`; anything else is an error. |
| Alignment | `general.alignment` (default 32); zero is an error, values not a multiple of 8 warn. |
| Tensors | At most 4 dimensions; element counts and byte sizes use checked arithmetic; duplicate names are an error. |
| Quantization | A mirrored `ggml_type` table (36 known ids at `llama.cpp` `de32b268c4ad55d45f3b7b16c029f4e077f4ba74`); unknown ids warn and leave the byte size unknown. Known types whose first dimension is not a multiple of the block size are an error. |
| Padding | Bytes between the tensor infos and the data section should be zero; non-zero padding warns. |
| Trailing bytes | Allowed and not reported. |

Caps keep adversarial files bounded before allocation:

| Limit | Value |
| --- | --- |
| Tensors per file | 2^20 |
| Metadata entries per file | 2^20 |
| Elements per metadata array | 2^20 |
| Metadata array nesting depth | 32 |
| Tensor dimensions | 4 |
| Metadata key length | 65535 bytes |
| Tensor name length | 64 bytes |

Every length read from the file is checked against the bytes that remain, and
size arithmetic is checked. Malformed input produces a typed error; it does not
panic. The test suite includes a truncation sweep and a random-bytes property
test. Non-fatal findings are stable kebab-case warning codes that never affect
the exit status; the full list is in [inspect](inspect.md).

## Inspect JSON contract

`hoard-gguf inspect --json` emits one versioned document:

```text
{
  "schema": 1,
  "file":     { path, size, version, byte_order, alignment, data_start },
  "model":    { architecture, name, quantization_version },
  "split":    null | { no, count, tensors_count },
  "metadata": { count, entries[] },
  "tensors":  { count, total_bytes, bytes_known, unknown_types, entries[] },
  "quant_mix": [ { type, type_id, tensors, elements, bytes, percent } ],
  "memory":   { tensor_bytes, file_bytes, overhead_bytes, padding_bytes },
  "warnings": [ { code, message } ]
}
```

The document is deterministic: metadata entries are in file order, tensors are
sorted by offset then name, `quant_mix` is sorted by bytes descending then type
id, and `warnings` are in parser emission order. Pretty output uses two spaces
and a trailing newline.

The shape is frozen at `schema: 1` and generated from the Rust types with
`schemars`; `schemas/inspect-v1.schema.json` is committed and a test fails when
it drifts. Breaking changes bump `schema`; compatible additions (a new warning
code, a new optional field) do not, and consumers should ignore unknown warning
codes. 64-bit counts and sizes are JSON integers, so a double-backed JSON
parser loses precision above 2^53; the caveat is documented in
[inspect](inspect.md).

## Manifest contract

A manifest v1 pins an artifact set to its source:

- `source`: the Hugging Face repository, a 40-character commit sha as
  `revision`, and `repoType` (`model`, `dataset`, or `space`).
- `files[]`: a POSIX-relative `path`, a `role` (`weights`, `mmproj`, `mtp`,
  `tokenizer`, `config`, `other`), an optional `group` for split shards, a
  positive `size`, the source `sha256` (the Hugging Face LFS oid), the hoard
  `blake3` of the bytes, and optional symmetric `siblings` for split weights.

The canonical encoding is the identity of a manifest: object keys sorted at
every level, two-space indent, LF line endings, exactly one trailing newline,
`files` sorted by path, sibling lists sorted, optional keys omitted when empty.
There are no timestamps or other non-deterministic fields, so the same input
always produces the same bytes.

```text
artifact_id = manifest_hash = "blake3:" <> hex(blake3(canonical_bytes))
```

The manifest hash is the artifact ID printed by `hoard manifest hash`. File
`blake3` digests identify content for the CAS; the `sha256` is source metadata.
The format, field rules, and trust model are documented in
[manifest](manifest.md).

## Transport (P2, partial)

P2 ships the fetch path for Hugging Face LFS/Xet files. The constraints from the
plan hold, and the remaining work is listed at the end.

- `hoard fetch <spec>` resolves a branch, tag, or commit to a 40-character
  commit sha before it lists files. The manifest records the sha, so a mutable
  ref cannot change between resolution and download.
- `hoard-hf` lists the repository tree at the resolved commit. A file must
  carry a published LFS SHA-256; a file without one is rejected before any
  request is made.
- `Transport::Auto` prefers Xet when the tree entry carries a Xet hash and
  falls back to plain HTTP when the Xet transfer fails for a transport reason.
  A digest or size mismatch is never retried on another transport.
- HTTP downloads write `<dest>.part`, verify the size and SHA-256, and rename.
  A truncated body keeps the part file for a Range resume; a digest mismatch
  removes it.
- Requests retry three times on 429, 5xx, connection, and timeout failures with
  exponential backoff and jitter. Redirects stop at the Hugging Face hosts, the
  Xet CAS host, or the configured endpoint host, with at most eight hops.
- The manifest is written after the bytes are hashed: `blake3` comes from the
  CAS entry, and `sha256` and `size` come from the source listing. The builder
  rejects any disagreement between the two, and the manifest write is atomic.
- Verification is fail-closed. `hoard fetch --manifest` checks the remote size
  and SHA-256 against the manifest before downloading and compares BLAKE3 after
  storing. `hoard verify` re-hashes CAS blobs or directory files. A mismatch
  exits 1 and no manifest is written.
- There is no trust-on-first-use cache. Reads re-hash the blob, so a poisoned
  store entry is rejected instead of used.

Remaining for P2: generic sources, a plan or dry-run mode, store garbage
collection, and a decision on chunk-level deduplication. The CAS is whole-file
by BLAKE3; Xet handles transfer deduplication.

## Nix integration (P3, partial)

The flake exports two helpers:

- `lib.fetchArtifact { pkgs, hoard, manifest, path }` builds one flat
  fixed-output derivation for one manifest entry.
- `lib.fetchArtifactSet { pkgs, hoard, manifest }` builds one FOD per entry and
  assembles the set as a symlink farm.

The flat-FOD decision follows from the hash formats. Nix fixed-output
derivations support sha256 and sha512, and manifest v1 records the source
sha256 for every file, so each file maps to one FOD with
`outputHashMode = "flat"`. The store path then contains exactly the file bytes,
and its name is `hoard-<first 12 characters of sha256>-<basename>`.
`hoard store` reproduces the same path with `nix-store --add-fixed sha256`, and
`hoard push` copies it with `nix copy`. A recursive NAR derivation would wrap
the bytes in a directory and force a whole set to rebuild when one file
changes, so the helper uses one FOD per file and a symlink farm for the set.

Signing belongs to the cache. Hoard holds no keys, and `hoard push` delegates to
`nix copy`; a cache that requires signatures needs Nix's signing configuration
on the pushing side.

`examples/nix/flake.nix` is a consumer that takes `github:meillaya/hoard` as an
input and calls `fetchArtifactSet` with
`examples/nix/models/stories15M-q8_0.json`. Local testing overrides the input
with `git+file:`. `scripts/check-nix-roundtrip.sh` builds the example, copies
its FODs to a temporary file cache, deletes them, rebuilds offline from that
cache, and compares the two output paths.

Remaining for P3: pin the manifest itself as a flake input by artifact ID, send
the helper pattern upstream, and decide whether Nix should learn BLAKE3
directly.

## Decisions

| Decision | Choice | Rationale |
| --- | --- | --- |
| Byte order | Little-endian only | GGUF is written little-endian in practice; a swapped version word almost always means a mis-built file. Rejecting with a precise message is safer than silently byte-swapping. |
| Versions | GGUF v2 and v3 only | v1 has an incompatible layout and is obsolete in `llama.cpp`; supporting it would add a second parser for no user benefit. |
| Revision | Hugging Face commit sha (40 hex) | A branch or tag is mutable; a commit sha makes manifests reproducible and matches the URL form Hugging Face serves. The `sha256:` prefix is not used because the revision is not a content hash. |
| Timestamps | None in manifests | The artifact ID must be a pure function of the content description. A `created_at` field, a download time, or a builder identity would all break that. |
| Digest usage | `sha256` from source, `blake3` for CAS | The source digest is checkable at download; BLAKE3 is fast, tree-hashable, and the identity hoard controls. Both are recorded so neither role is overloaded. |
| CAS granularity | Whole-file blobs keyed by BLAKE3 | Manifest v1 already names each file by BLAKE3, so file-level storage needs no new index. Xet deduplicates transfer; a local chunk store would add a second identity and a garbage-collection problem. Revisit if local dedupe matters. |
| Nix FOD shape | One flat FOD per file, symlink farm for the set | FODs support sha256 and the manifest records it. Flat mode makes the store path the file bytes, so `hoard store` reproduces it and each file caches on its own. |
| Transport order | Xet when available, HTTP fallback | Xet deduplicates transfer and fits the Hub's large-file path; HTTP keeps a Range-resume path that works without Xet. |
| Mutable refs | Resolve to a commit sha before download | The manifest records the commit, so a branch or tag that moves later cannot change the artifact identity. |
| Token source | Environment only | A token flag would land in shell history and process listings; `HF_TOKEN` and `HUGGING_FACE_HUB_TOKEN` keep it out of both. |
| Signing | Delegate to `nix copy` and the cache | Hoard proves content identity, not publisher identity. Keeping keys out of hoard avoids a second signing policy. |
| JSON schema | Frozen `schema: 1`, committed schema file | Consumers (`jq`, dashboards, Nix) need a stable contract. Drift is caught by a schema-equality test, and breaking changes require a version bump. |
| Memory mapping | Zero-copy view, one audited `unsafe` | Inspecting a multi-gigabyte file should be cheap. Keeping the only `unsafe` in one documented function makes the workspace lint meaningful. |
| Binary split | `hoard` and `hoard-gguf` are separate | The format tool is useful without the artifact layer, and the artifact layer must not grow a GGUF dependency. |
| Test runner | `cargo test`, not `nextest` | One toolchain, no extra install step in CI or the Nix build; the suite is small enough that process-level parallelism is enough. `cargo-nextest` can be layered on later without changing the tests. |
| Flake scope | nixpkgs input only, no `flake-utils`/devenv | Three systems are enumerated by hand in one small `flake.nix`; fewer moving inputs means fewer lock updates and less eval machinery. |
| crates.io names | Publish `hoard-gguf` first; `hoard`/`hoard-cli` publication deferred | The short names `hoard` and `hoard-cli` are likely taken on crates.io, so the release plan publishes the standalone format crate first and resolves the CLI package name before its first release. The binary names are unaffected. |
| Development shell | `devenv.nix` is the single definition; the flake delegates to it; rust-overlay honors `rust-toolchain.toml` | `devenv shell` and `nix develop --impure` share one environment and one Rust pin. Plain cargo stays available as the secondary path. |
| Intel macOS | Not in the flake system list | nixpkgs 26.11 dropped `x86_64-darwin`; see [limits and non-goals](limitations.md). |
