# Limits and non-goals

hoard 0.1.0 inspects GGUF model files, fetches and verifies Hugging Face
artifacts, and turns a pinned manifest into flat Nix store paths. Everything
below is out of scope for this release; some of it is planned, and the roadmap
says which.

## Format

- hoard accepts little-endian files only. Big-endian GGUF files are rejected
  with a dedicated error instead of being byte-swapped. In practice GGUF is
  written little-endian; a swapped version word usually means a file built by a
  broken tool.
- hoard reads GGUF v2 and v3 only. Version 1 used a different metadata layout
  and is not supported; other versions are rejected with an `unsupported-version`
  error.
- The inspect report records shapes, types, element counts, and byte sizes. It
  never reads, dequantizes, or validates tensor values, and there is no `dump`,
  `verify-data`, or `convert` command. Unknown quantization types are
  inspectable (with a warning), but their byte size is unknown.
- hoard cannot create, edit, or re-pack a GGUF file. There is no split/merge
  command: split models can be inspected one part at a time, and the `split.*`
  metadata is reported, but the parts are not reassembled or cross-checked
  against each other.
- Parsing validates structure. No command answers "is this GGUF conformant to
  the full `llama.cpp` spec", and none compares against `llama.cpp`'s reader;
  the differential harness is planned for P1, see [testing](testing.md).
- hoard accepts files as-is when they are structurally valid; it does not
  rewrite metadata order, alignment, or padding.
- Trailing bytes after the last tensor are allowed and not reported.
- Multimodal projector files can be inspected, but their role in a model set is
  only expressible in manifests, not interpreted.

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

No file-size cap applies because hoard memory-maps the file instead of reading
it into memory. A file that is larger than the address space cannot be mapped
and fails with an I/O error.

## Inspect report

- `metadata.entries[].array.sample` contains at most three scalar elements;
  nested arrays are not recursed at all. Use the GGUF parser API if you need
  array values, and do not expect the report to be a full metadata dump.
- The report is frozen at `schema: 1`; see [inspect](inspect.md) for ordering,
  precision, and compatibility rules.
- The report counts tensors and bytes as 64-bit JSON integers. A consumer that
  decodes JSON numbers into doubles loses precision above 2^53.
- The report describes the bytes on disk. It does not prove where they came
  from.

## Commands

0.1.0 ships two binaries:

| Binary | Commands |
| --- | --- |
| `hoard-gguf` | `inspect`, `schema` |
| `hoard` | `hash`, `manifest validate`, `manifest fmt`, `manifest hash`, `manifest schema`, `fetch`, `verify`, `store`, `push` |

The commands split by network use:

- `hoard-gguf`, `hoard hash`, and `hoard manifest` read local files and print
  output. They write nothing and open no network connections.
- `hoard fetch` is the only command that opens a network connection. It writes
  the CAS and optional `--out` files, and for a spec it writes a manifest.
- `hoard verify` and `hoard store` read the CAS. `hoard push` runs `nix copy`
  against the destination URI.

Not shipped:

- Generic sources. Fetch works against Hugging Face only, and only for files
  whose tree entry carries an LFS object id. There is no generic HTTPS URL,
  local directory, S3, or Git source, and no transfer of a non-LFS file.
- Manifest editing. `hoard manifest fmt` writes the canonical form to stdout;
  it does not rewrite the input file. `hoard fetch` generates a manifest from a
  Hugging Face listing; nothing builds a manifest from a local directory.
- A configuration file. Configuration lives in arguments, manifest fields, and
  the environment variables listed in [usage](usage.md#environment-variables).
- A store GC or prune command. The CAS has no index and no removals.
- Parallel downloads. `hoard fetch` downloads selected files one at a time.
- A plan or dry-run mode, and any progress user interface beyond the stderr
  lines.

## Fetch and transport

- Fetch supports Hugging Face LFS/Xet files only. A file without a published
  LFS SHA-256 is rejected before any request is made.
- Public repositories are tested. `HF_TOKEN` and `HUGGING_FACE_HUB_TOKEN` cover
  gated and private repositories, but the test suite does not exercise a private
  repository.
- Transports are Xet and plain HTTP. There is no FTP, S3, or Git transport.
- HTTP resume is a Range request against a kept `<dest>.part` file. Xet resume
  and shard caching are delegated to `hf-xet` and `HOARD_XET_CACHE`.
- The CAS is whole-file and keyed by BLAKE3. There is no content-defined
  chunking and no local chunk-level dedupe; Xet deduplicates transfer.
- Fetch runs resolve a branch or tag to a commit sha first and record the sha.
  A manifest revision is always a 40-character commit sha; hoard cannot fetch a
  manifest itself, so a manifest has to arrive through a channel you trust.
- `--offline` applies only with `--manifest`. A spec has to be resolved online
  before any byte can be cached.
- Downloads verify the source SHA-256 and the computed BLAKE3. There is no
  option to accept a mismatch, and no trust-on-first-use mode.

## Manifests

- `source.kind` accepts only `"huggingface"`; a local directory or a generic
  HTTPS URL is not expressible in v1.
- A manifest cannot mix files from different repositories or revisions.
- A manifest is plain canonical JSON with no signature or attestation. The
  BLAKE3 artifact ID is an identity, not a signature: it proves that two
  manifests are identical, not who wrote them. Signing is not implemented.
- A manifest records digests, not file contents, so it cannot reconstruct a
  file; the bytes must come from somewhere else.
- Manifests carry no timestamps, so the artifact ID is deterministic. They
  cannot record when or by whom a file was produced.
- A manifest has no HTTP URLs: paths are relative to the pinned source
  revision, and there is no per-file URL override.
- hoard reads schema v1 only: unknown fields and unknown schema versions are
  rejected, so forward-compatible extensions require a schema bump.

## Nix

- The flake exports `lib.fetchArtifact` and `lib.fetchArtifactSet`, which read a
  local manifest v1 file and build flat fixed-output derivations. Each helper
  takes the hoard package and the manifest as arguments, so the consumer owns
  both inputs.
- The manifest is not pinned by artifact ID as a flake input yet. You pass a
  path, and pinning that file is your job.
- One flat FOD per file and a symlink farm for the set. There is no recursive
  NAR derivation and no single derivation that carries a whole set.
- Nix fixed-output derivations use sha256 and sha512, so the flow uses the
  manifest `sha256`. Nix does not check BLAKE3 directly.
- Hoard holds no signing keys. `hoard push` delegates upload and signing to
  `nix copy`, and the destination cache decides its signature policy.
- The first build of a file needs network access. Later builds use the binary
  cache or the local store, and the round-trip script checks that an offline
  rebuild produces the same path.

## Platforms

- Development shell coverage: devenv is validated on `x86_64-linux`. The
  `aarch64-linux` and `aarch64-darwin` flake outputs are evaluated, not built
  locally. A system or rustup Rust 1.99.1 remains a supported secondary path.

- Linux x86_64 is the tested platform. The repository is developed and tested
  on `x86_64-linux` only. The flake also exposes `aarch64-linux` and
  `aarch64-darwin` outputs because they evaluate, but they are not exercised
  locally.
- The flake system list is `x86_64-linux`, `aarch64-linux`, and
  `aarch64-darwin`. nixpkgs 26.11 dropped `x86_64-darwin`, so Intel macOS is not
  in the flake system list: there is no devShell or package guarantee there.
  Cargo builds may still work if you install Rust by other means.
- hoard has no Windows target. The crates use `memmap2` and have no
  Windows-specific code paths, but the flake does not cover Windows and the
  test suite is not run there. The CLI does not promise Windows support in
  0.1.0.
- hoard has no 32-bit targets. Counts and offsets are 64-bit, and 32-bit
  platforms have not been considered.

## Security boundaries

See [security](security.md) for the threat model. hoard validates file
structure and content hashes; it does not vouch for the publisher. A
memory-mapped file truncated by another process can abort the inspecting
process. `hoard fetch` trusts the Hub for the repository listing and the
source `sha256`, and the pinned commit and TLS bound that trust. Hoard keeps no
trust-on-first-use state, and cache signing is delegated to Nix.
