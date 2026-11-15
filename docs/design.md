# Design

hoard is a set of tools for moving large model files without trusting the
transport. It starts from the GGUF container format used by `llama.cpp`:
`hoard-gguf` answers "what is in this file", and `hoard` answers "which bytes
make up this artifact set, and what is their identity".

The long-term shape is five layers:

| Layer | Shipped in 0.1.0 | Responsibility |
| --- | --- | --- |
| Format | yes | `hoard-gguf`: strict GGUF parsing, inspect report, warning codes. |
| Artifact | yes (library + CLI) | `hoard-core`: manifest v1, canonical JSON, validation, streaming hashes. |
| Tooling | yes | `hoard` and `hoard-gguf` binaries: commands, JSON output, exit codes. |
| Transport | no (P2) | Download from the source, verify, populate the content-addressed store. |
| Nix integration | no (P3) | `fetchArtifact`-style fixed-output derivations over a pinned manifest. |

Everything in 0.1.0 is read-only and offline: commands read local files and
never write a model file, touch the network, or mutate a manifest in place.

## Crates and boundaries

| Crate | Binary | Depends on | Boundary |
| --- | --- | --- | --- |
| `hoard-gguf` | `hoard-gguf` | `memmap2`, `serde`, `serde_json`, `schemars`, `thiserror`, optional `clap`/`anyhow` | GGUF only. Does not know about manifests, hashes, the network, or other artifacts. |
| `hoard-core` | none | `blake3`, `sha2`, `hex`, `serde`, `serde_json`, `schemars`, `thiserror` | Manifest and hashing only. Does not link `hoard-gguf`; a manifest can describe any file set. |
| `hoard-cli` | `hoard` | `hoard-core`, `anyhow`, `clap`, `serde_json` | Command surface. Does not link `hoard-gguf`; the two binaries stay independent. |

The dependency direction is deliberate. The format layer works on its own, the
artifact layer has no GGUF-specific types, and the manifest hash does not change
when GGUF parsing changes. A future transport layer belongs in `hoard-core` (or
a sibling crate) and must depend on the artifact layer, not on the CLI.

Both binaries are thin: argument parsing, human formatting, and error context.
The libraries hold the logic, and their unit tests exercise it.

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

## Planned transport (P2)

P2 adds the commands that move and verify the bytes: resolve a manifest for a
Hugging Face repository at a commit, download the files, verify them, and store
them by BLAKE3. The design constraints:

- The manifest is written after the bytes are hashed, so `blake3` always
  describes downloaded content. The `sha256` comes from the source metadata
  (Hugging Face LFS oid) and is checked against the response.
- A resume path writes partial downloads to a temp file and renames atomically;
  a killed process must never leave a partial file at its final CAS path.
- Verification is fail-closed: any digest mismatch, size mismatch, or missing
  file fails the command and reports the file, expected hash, and observed
  hash. There is no "trust on first use" cache.
- Transport is separate from format: `hoard fetch` writes a manifest and CAS
  entries, and `hoard-gguf inspect` remains a local, offline tool.

## Planned Nix integration (P3)

Nix users need the artifact set to participate in builds and binary caches.
A fixed-output derivation can pin one file, and a hoard manifest supplies the
hashes. The sketch below is not shipped in 0.1.0:

```nix
# fetchArtifact: one manifest entry as a Nix fixed-output derivation.
# The manifest is itself an input (pinned by its artifact ID); the FOD is
# checked by Nix, so a build cannot observe bytes that hoard would reject.
{ fetchurl }:

{ manifest, path }:
let
  entry = builtins.head (builtins.filter (file: file.path == path) manifest.files);
  # Nix FODs support sha256/sha512; the manifest's sha256 is the source
  # (Hugging Face LFS) digest. The blake3 remains the CAS identity.
  hash = "sha256:${entry.sha256}";
in
fetchurl {
  name = baseNameOf entry.path;
  url = "https://huggingface.co/${manifest.source.repo}/resolve/${manifest.source.revision}/${entry.path}";
  inherit hash; # Nix checks the bytes before they enter the store
}
```

The open P3 questions are: pinning the manifest itself as a flake input (by
artifact ID) versus a `fetchArtifact` that takes a pinned manifest hash, whether
to teach Nix BLAKE3 directly, and how a multi-file artifact set becomes one
derivation (symlink farm or output directory). 0.1.0 promises none of it.

## Decisions

| Decision | Choice | Rationale |
| --- | --- | --- |
| Byte order | Little-endian only | GGUF is written little-endian in practice; a swapped version word almost always means a mis-built file. Rejecting with a precise message is safer than silently byte-swapping. |
| Versions | GGUF v2 and v3 only | v1 has an incompatible layout and is obsolete in `llama.cpp`; supporting it would add a second parser for no user benefit. |
| Revision | Hugging Face commit sha (40 hex) | A branch or tag is mutable; a commit sha makes manifests reproducible and matches the URL form Hugging Face serves. The `sha256:` prefix is not used because the revision is not a content hash. |
| Timestamps | None in manifests | The artifact ID must be a pure function of the content description. A `created_at` field, a download time, or a builder identity would all break that. |
| Digest usage | `sha256` from source, `blake3` for CAS | The source digest is checkable at download; BLAKE3 is fast, tree-hashable, and the identity hoard controls. Both are recorded so neither role is overloaded. |
| JSON schema | Frozen `schema: 1`, committed schema file | Consumers (`jq`, dashboards, Nix) need a stable contract. Drift is caught by a schema-equality test, and breaking changes require a version bump. |
| Memory mapping | Zero-copy view, one audited `unsafe` | Inspecting a multi-gigabyte file should be cheap. Keeping the only `unsafe` in one documented function makes the workspace lint meaningful. |
| Binary split | `hoard` and `hoard-gguf` are separate | The format tool is useful without the artifact layer, and the artifact layer must not grow a GGUF dependency. |
| Test runner | `cargo test`, not `nextest` | One toolchain, no extra install step in CI or the Nix build; the suite is small enough that process-level parallelism is enough. `cargo-nextest` can be layered on later without changing the tests. |
| Flake scope | nixpkgs input only, no `flake-utils`/devenv | Three systems are enumerated by hand in one small `flake.nix`; fewer moving inputs means fewer lock updates and less eval machinery. |
| crates.io names | Publish `hoard-gguf` first; `hoard`/`hoard-cli` publication deferred | The short names `hoard` and `hoard-cli` are likely taken on crates.io, so the release plan publishes the standalone format crate first and resolves the CLI package name before its first release. The binary names are unaffected. |
| Development shell | `devenv.nix` is the single definition; the flake delegates to it; rust-overlay honors `rust-toolchain.toml` | `devenv shell` and `nix develop --impure` share one environment and one Rust pin. Plain cargo stays available as the secondary path. |
| Intel macOS | Not in the flake system list | nixpkgs 26.11 dropped `x86_64-darwin`; see [limits and non-goals](limitations.md). |
