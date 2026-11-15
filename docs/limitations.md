# Limits and non-goals

hoard 0.1.0 is a read-only inspection and identity tool for GGUF model files.
It does not download, convert, split, merge, quantize, or serve models.
Everything below is out of scope for this release; some of it is planned, and
the roadmap says which.

## Format

- **Little-endian only.** Big-endian GGUF files are rejected with a dedicated
  error instead of being byte-swapped. In practice GGUF is written
  little-endian; a swapped version word usually means a file built by a broken
  tool.
- **GGUF v2 and v3 only.** Version 1 used a different metadata layout and is not
  supported; other versions are rejected with an `unsupported-version` error.
- **No tensor decoding.** The inspect report records shapes, types, element
  counts, and byte sizes. It never reads, dequantizes, or validates tensor
  *values*, and there is no `dump`, `verify-data`, or `convert` command. Unknown
  quantization types are inspectable (with a warning) but their byte size is
  unknown.
- **No writer.** hoard cannot create, edit, or re-pack a GGUF file. There is no
  split/merge command: split models can be inspected one part at a time, and
  the `split.*` metadata is reported, but the parts are not reassembled or
  cross-checked against each other.
- **No full-file validator.** Parsing validates structure, but there is no
  command that answers "is this GGUF conformant to the full `llama.cpp` spec" or
  that compares against `llama.cpp`'s reader (the differential harness is
  planned for P1, see [testing](testing.md)).
- **No writer-side canonicalization.** Files are accepted as-is if structurally
  valid; hoard does not rewrite metadata order, alignment, or padding.
- **Trailing bytes are allowed** after the last tensor and are not reported.
- **No mmproj semantics.** Multimodal projector files can be inspected, but
  their role in a model set is only expressible in manifests, not interpreted.

## Limits on input

The parser bounds everything it reads so that adversarial files cannot exhaust
memory:

| Limit | Value |
| --- | --- |
| Tensors per file | 2^20 |
| Metadata entries per file | 2^20 |
| Elements per metadata array | 2^20 |
| Metadata array nesting depth | 32 |
| Tensor dimensions | 4 |
| Metadata key length | 65535 bytes |
| Tensor name length | 64 bytes (longer names warn) |
| Array sampling in JSON | at most 3 scalar elements |

There is no file-size cap: the file is memory-mapped, not read into memory.
A file that is larger than the address space cannot be mapped and fails with an
I/O error.

## Inspect report

- **Arrays are sampled.** `metadata.entries[].array.sample` contains at most
  three scalar elements; nested arrays are not recursed at all. Use the GGUF
  parser API if you need array values, and do not expect the report to be a full
  metadata dump.
- **One schema version.** The report is frozen at `schema: 1`; see
  [inspect](inspect.md) for ordering, precision, and compatibility rules.
- **64-bit integers.** Fields count tensors and bytes as JSON integers. A
  consumer that decodes JSON numbers into doubles loses precision above 2^53.
- **No signatures.** The report describes the bytes on disk. It does not prove
  where they came from.

## Commands

0.1.0 ships two binaries:

| Binary | Commands |
| --- | --- |
| `hoard-gguf` | `inspect`, `schema` |
| `hoard` | `hash`, `manifest validate`, `manifest fmt`, `manifest hash`, `manifest schema` |

Not shipped:

- **Fetching.** No `fetch`, `download`, `pull`, or `sync` command. Files must be
  downloaded by the user; hoard only hashes and validates them once they are
  local. Transport is planned for P2.
- **Artifact storage.** No content-addressed store, no cache, no
  `verify`-against-CAS command. The manifests record BLAKE3 digests, but hoard
  does not yet build a store from them.
- **Transport.** No Hugging Face API client, no HTTP, no retries, no resume, no
  authentication. Everything in 0.1.0 works offline.
- **Manifest editing.** `hoard manifest fmt` writes the canonical form to
  stdout; it does not rewrite the input file. There is no command that creates a
  manifest from a directory or from the network.
- **Environment variables.** Neither binary reads configuration from the
  environment in 0.1.0; all input is command-line arguments and file paths.

## Manifests

- **Hugging Face only.** `source.kind` accepts only `"huggingface"`; a local
  directory or a generic HTTPS URL is not expressible in v1.
- **One source per manifest.** A manifest cannot mix files from different
  repositories or revisions.
- **No signatures or attestations.** A manifest is plain canonical JSON. The
  BLAKE3 artifact ID is an identity, not a signature: it proves that two
  manifests are identical, not who wrote them. Signing is not implemented.
- **No file contents, only digests.** A manifest cannot be used to reconstruct a
  file; the bytes must come from somewhere else.
- **No timestamps.** Deliberately excluded so the artifact ID is deterministic;
  manifests cannot record when or by whom a file was produced.
- **No HTTP URLs.** Paths are relative to the pinned source revision; there is
  no per-file URL override.
- **Schema v1 only.** Unknown fields and unknown schema versions are rejected, so
  forward-compatible extensions require a schema bump.

## Nix

- **No artifact-layer commands in the flake.** `nix build .#default` builds the
  two binaries, but there is no `fetchArtifact`, `fetchManifest`, or model-set
  derivation yet. The integration is planned for P3; the sketch is in
  [design](design.md).
- **No binary cache or store integration.** hoard does not produce Nix store
  paths or NARs in 0.1.0.

## Platforms

- **Linux x86_64 is the tested platform.** The repository is developed and
  tested on `x86_64-linux` only. The flake also exposes `aarch64-linux` and
  `aarch64-darwin` outputs because they evaluate, but they are not exercised
  locally.
- **Flake systems: `x86_64-linux`, `aarch64-linux`, `aarch64-darwin`.**
  nixpkgs 26.11 dropped `x86_64-darwin`, so Intel macOS is not in the flake
  system list: there is no devShell or package guarantee there. Cargo builds may
  still work if you install Rust by other means.
- **No Windows target.** The crates use `memmap2` and have no Windows-specific
  code paths, but the flake does not cover Windows and the test suite is not run
  there. The CLI does not promise Windows support in 0.1.0.
- **No 32-bit targets.** Counts and offsets are 64-bit; 32-bit platforms have not
  been considered.

## Security boundaries

See [security](security.md) for the threat model. In short: hoard validates file
structure and content hashes, not the trustworthiness of a publisher, and a
memory-mapped file truncated by another process can abort the inspecting
process.
