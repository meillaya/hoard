# Security model

hoard reads model files that are usually large, downloaded from the internet,
and not controlled by the person inspecting them. Since 0.1.0 it also downloads
artifacts from Hugging Face and keeps verified bytes in a local store. The
security model covers the inspecting process, the fetch path, and the identity
of the artifacts hoard reports. Model contents are out of scope.

## Assets

| Asset | Why it matters |
| --- | --- |
| Process integrity | Parsing an untrusted GGUF must not panic, read out of bounds, or loop forever. |
| Memory safety | One `unsafe` block exists; a bug there is a memory-safety bug, worse than a wrong answer. |
| Artifact identity | The manifest hash is the name downstream systems use. A wrong hash silently breaks verification. |
| Manifest semantics | A manifest that validates must not contain path traversal, ambiguous digests, or broken shard relations. |
| Stored bytes | The CAS is trusted only after re-hashing; a corrupt or attacker-written blob must never be used. |
| Transport integrity | Downloaded bytes must match the pinned manifest, and a hostile response must not substitute content. |
| Token confidentiality | `HF_TOKEN` grants access to gated and private repositories. It must stay out of logs, error messages, and store paths. |
| Supply-chain integrity | A compromised dependency or build step compromises every hash hoard emits. |

## Adversaries and threats

- A malicious or corrupted GGUF file can lie about counts and lengths, nest
  metadata arrays, declare tensors that overlap or point past the end of the
  file, use an unknown quantization id, or contain invalid UTF-8. The goal
  could be a crash, memory exhaustion, integer overflow, an out-of-bounds read,
  or a report that misleads a consumer.
- Another process, or the same user, can truncate or replace the underlying
  file while `hoard-gguf` has it memory-mapped. Touching the lost pages raises
  `SIGBUS`.
- A hand-crafted manifest can try path traversal (`../`, absolute paths),
  duplicate or unsorted entries, digest strings that are not digests, broken
  sibling relations, or a source revision that is a branch. A consumer that
  parses without validating would trust it.
- A malicious or vulnerable crates.io dependency, a typosquat, a yanked
  release, or a non-crates.io source compromises the binaries.
- Untrusted Hub responses. The Hub, or an endpoint reached through
  `HF_ENDPOINT`, returns a file listing, redirects, and file bodies. Threats
  include wrong bytes for a path, a redirect to an unrelated host, a truncated
  body, a mutable ref that resolves to a different commit, a listing that omits
  a file, and a server that lies about size or digest.
- Token exposure. A token can leak into shell history, process listings, error
  messages, logs, or a build log.
- Path traversal from remote names. A repository path or manifest path such as
  `../../.ssh/authorized_keys` could escape the output directory when hoard
  materializes files.
- CAS poisoning. A local process or a crashed write can leave bytes under the
  wrong BLAKE3 name; a read must not trust the name.
- A compromised local store. A partial write, a disk error, or an interrupted
  materialization can leave wrong bytes at a destination.
- The Nix FOD boundary. A fixed-output derivation runs a network fetch inside
  the build sandbox, and a binary cache signature policy decides whether pushed
  paths are accepted elsewhere.

## Mitigations

### Bounds-checked parser and caps

- Every length read from the file is checked against the bytes that remain
  before it is used. Every count is checked against a cap before allocation.
- All element-count and byte-size arithmetic uses checked operations.
- Metadata arrays are capped at 2^20 elements and 32 nesting levels, and files
  are capped at 2^20 metadata entries. Tensor counts are capped at 2^20, and
  tensor names and metadata keys have length limits. The nesting cap matters
  because parsed arrays are recursively owned: without it, dropping a crafted
  deeply-nested value would abort the process with a stack overflow.
- The inspect report samples arrays and never recurses into nested arrays, so
  report size is bounded by the number of entries, not by declared element
  counts.
- Tensor overlap detection sorts tensor offsets (`O(n log n)`) rather than
  comparing every pair.

### One audited unsafe operation

The whole workspace denies `unsafe_code`; the single exception is
`memmap2::Mmap::map` in `crates/hoard-gguf/src/file.rs`, under a documented
`#[allow(unsafe_code)]` with a SAFETY comment. The mapping is read-only and
lives as long as the `GgufFile`; all parsing works on a borrowed `&[u8]`.

### Verified fetch

- `hoard fetch` resolves a mutable ref to a commit sha once and records the sha
  in the manifest. The download URL uses the resolved sha, not the branch.
- The client requires a published LFS SHA-256 for every file and rejects a file
  without one before any request.
- Each download is written to `<dest>.part` and checked against the expected
  size and SHA-256; only then is it renamed. BLAKE3 is computed while the file
  is copied into the CAS, so a manifest `blake3` always describes stored bytes.
- `hoard fetch --manifest` checks the remote size and SHA-256 against the
  manifest before it downloads a missing file, and compares BLAKE3 after
  storing. A mismatch exits 1 and no manifest is written.
- A digest or size mismatch is not retried, and `auto` does not fall back from
  Xet to HTTP for it: the bytes were wrong.
- The manifest is built after every selected file verifies and is written
  atomically.
- There is no trust-on-first-use cache. Reads re-hash the blob, so a poisoned
  store entry is rejected instead of used.

### Network policy

- HTTPS through rustls for public hosts. Plain HTTP is accepted only on the
  configured `HF_ENDPOINT` host, which is how a local mirror or test server
  works.
- Redirects follow at most 8 hops and only to `huggingface.co`, hosts under
  `hf.co`, the Xet CAS host, or the configured endpoint host. A redirect
  elsewhere fails the request.
- Requests retry on 429, 5xx, connection, and timeout failures, with
  exponential backoff and jitter. Digest and size failures are not retried.
- The HTTP client sets a 30 second connect timeout.

### Token handling

- The token comes from `HF_TOKEN` or `HUGGING_FACE_HUB_TOKEN`. There is no flag
  and no configuration file, so it stays out of the process argument list and
  shell history.
- The client sends the token as an `Authorization: Bearer` header. It is
  redacted from `Debug` output and never included in an error message.
- The Nix `fetchArtifact` helper declares `HF_TOKEN` as an impure environment
  variable. Nix passes the value to the build process and does not record it in
  the derivation.
- Hoard does not write the token to a file and does not keep it after the
  command exits.

### Path handling

- A manifest path is validated before use: relative, no `..` segments, no
  leading `/`, no backslashes, no NUL, no empty segments, at most 1024 bytes.
- `hoard fetch` builds file URLs from the repository tree path; empty, `.` and
  `..` components are rejected before a URL is built.
- Materialization joins the validated manifest path to the output directory. It
  refuses to write through a symlink and refuses a directory at the destination.
- CAS blob paths are derived from digest hex only, so no manifest field can
  point outside `<root>/blobs`.

### Fail-closed validation

- Parsing either produces a validated `Gguf` or a typed error. There is no
  "best effort" mode, and warnings never suppress errors that affect structure.
- Manifest validation is separate from parsing. `manifest validate`,
  `manifest hash`, `fetch`, `verify`, `store`, and `push` validate the manifest
  before using it.
- `manifest fmt --check` fails when a file is not canonical, so a repository
  cannot drift silently from its documented identity.
- Fetch verification is fail-closed: a mismatch rejects the file, and the
  manifest file appears only after every selected file verifies.

### CAS integrity

- Stored bytes are named by BLAKE3 and verified on read. `verify`, `store`,
  `push`, and `--out` re-hash a blob and check size, SHA-256, and BLAKE3 before
  use.
- Adds are atomic. The blob path appears only after a fsynced temporary file is
  renamed into place, and a failed put leaves no blob.
- The store has no index, so there is no index to poison. The name is the
  content digest.
- The store lives in the user's cache directory under the user's permissions.
  Same-user attackers are out of scope.
- `hoard store` verifies the CAS entry, and then `nix-store --add-fixed sha256`
  re-hashes the bytes before adding them to the Nix store.

### Nix fixed-output boundary

- Each file becomes a flat FOD whose declared hash is the manifest `sha256`.
  Nix checks the downloaded bytes against `outputHash` before the path enters
  the store, so a build cannot observe bytes that hoard would reject.
- `hoard push` delegates upload and signing to `nix copy`. Hoard holds no
  private keys; a cache that requires signatures needs the Nix-side signing
  configuration.
- The round-trip check turns signature checking off only for its own temporary
  local file cache, which the script creates and deletes.

### Supply chain

- `Cargo.lock` is committed and CI builds with a pinned Rust toolchain
  (`rust-toolchain.toml`, and `dtolnay/rust-toolchain@1.99.1`).
- `cargo-deny` runs in CI with advisories, bans, licenses, and sources checks
  ([`deny.toml`](../deny.toml)). Dependencies must come from crates.io.
- The fetch path adds `reqwest`, `hf-xet`, and their trees (`rustls`,
  `aws-lc-rs`). They are locked in `Cargo.lock` and covered by the same deny
  checks.
- `devenv` and `rust-overlay` are pinned by full revisions in `flake.lock` and
  `devenv.lock`; the development shell is not a floating environment.
- `flake.lock` pins nixpkgs, and the Nix package resolves dependencies through
  `Cargo.lock` (`importCargoLock`), so the Nix build uses exactly the locked
  crates, each verified against its lockfile checksum.
- Release artefacts must be built from a clean checkout in CI. Signing and
  provenance attestations are not implemented in 0.1.0 (see below).

### Residual risk: `SIGBUS` from concurrent truncation

Another process can truncate the file while it is mapped. Touching the removed
pages raises `SIGBUS` and terminates the process, and no portable way exists to
catch and recover from it. Mitigations:

- hoard never writes to model files and holds the mapping only while inspecting.
- The limitation is documented on `GgufFile` and here, so callers can avoid
  inspecting files that are being written concurrently.
- Hashing streams the file with `read` instead of mapping it, so a shrinking
  file cannot fault the process; it hashes the bytes that are actually
  readable. The printed `size` comes from a stat taken before hashing, so it can
  disagree with the hashed bytes if another process truncates the file
  concurrently.

This is a denial-of-service risk only; it cannot corrupt output that was
already printed.

## Out of scope

- Publisher trust. A manifest hash proves two manifests are identical; it does
  not prove that the publisher is honest. A malicious Hugging Face repository
  can publish files whose `sha256` and `blake3` match whatever it wants. The
  first fetch trusts the Hub listing for size and `sha256`; the manifest then
  pins those values.
- Endpoint trust. An `HF_ENDPOINT` mirror or a compromised TLS endpoint sees
  which files you fetch and can return a different listing. Pinned digests catch
  altered bytes; they do not hide the request metadata.
- Model safety. hoard does not decode tensor values and does not execute model
  code, so it cannot tell whether a model's content is safe or correct.
  Vulnerabilities in `llama.cpp` or in a model loader that later consumes the
  bytes are outside hoard.
- Signing and attestations. Manifest v1 has no signature, timestamp, or
  provenance attestation. CI does not attest releases in 0.1.0.
- Same-user attackers. An attacker with the same OS privileges as the running
  process can read and replace the files being inspected, change binaries, read
  the token from the environment, or attach to the process. `SIGBUS` is one
  symptom; the whole local threat model is out of scope.
- Filesystem I/O failures. A network filesystem that returns I/O errors on
  mapped pages can also fault the process; this is treated like truncation.
- Traffic analysis. `hoard fetch` sends the repository, revision, file paths,
  and sizes to the endpoint. TLS hides them from the network path, not from the
  endpoint or a configured mirror.
- Terminal escape injection. Error and warning messages can contain strings
  copied from the input (metadata keys, tensor names, file paths). Do not render
  hoard output in a terminal or log viewer that interprets escape sequences
  without escaping it first. The report's JSON strings are escaped normally, but
  the human-readable output is meant for humans.

## Future work and residual risks

| Risk | Plan |
| --- | --- |
| Generic or non-Hugging Face sources | Keep the digest policy per source and never relax verification for convenience. |
| Chunk-level dedupe | Decide whether the CAS moves below file granularity. BLAKE3 identity and re-hash on read stay either way. |
| Store GC and prune | Add a plan and prune path that cannot delete a blob a manifest still references. |
| Nix helper upstreaming | Send the flat-FOD helper pattern upstream and keep the round-trip check as the gate. |
| Parser assurance | The differential harness and fuzzing targets remain planned; they gate parser changes, not the fetch path. |
| Publication and signing | Resolve crates.io names and decide whether signing belongs in the format, the transport, or the cache. |

## Operational guidance

- Keep tokens in the environment, not in a command line or a file you commit.
- Fetch by pinned commit when you can, and review a manifest you did not write.
- Run `hoard verify --manifest <FILE> --store` before a build that relies on the
  CAS.
- Use `--offline` for repeat builds once the files are cached. It removes the
  network from the build path.
- Treat the CAS as a cache, not a backup. It has no GC; delete the store root
  deliberately.
- Configure cache signing keys in Nix for a cache that requires them. Do not
  add signing keys to hoard.
- Treat manifests as code: review changes to `source.revision`, `sha256`,
  `blake3`, and `siblings` like you review dependency changes.
- Do not truncate or rewrite a model file while any process is inspecting it.
- Check the `warnings` array before trusting an inspect report for model
  integrity decisions; overlapping tensors and unknown quantization types are
  accepted on purpose and reported there.
- Run `cargo deny check` before publishing or tagging a release; CI does it on
  every pull request.
