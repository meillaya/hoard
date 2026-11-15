# Security model

hoard reads model files that are usually large, downloaded from the internet,
and not controlled by the person inspecting them. The security model is about
the *inspecting process* and the *identity of artifacts*, not about the models
themselves.

## Assets

| Asset | Why it matters |
| --- | --- |
| Process integrity | Parsing an untrusted GGUF must not panic, read out of bounds, or loop forever. |
| Memory safety | One `unsafe` block exists; a bug there is a memory-safety bug, not just a wrong answer. |
| Artifact identity | The manifest hash is the name downstream systems use. A wrong hash silently breaks verification. |
| Manifest semantics | A manifest that validates must not contain path traversal, ambiguous digests, or broken shard relations. |
| Supply-chain integrity | A compromised dependency or build step compromises every hash hoard emits. |

## Adversaries and threats in 0.1.0

- **A malicious or corrupted GGUF file.** It can lie about counts and lengths,
  nest metadata arrays, declare tensors that overlap or point past the end of
  the file, use an unknown quantization id, or contain invalid UTF-8. The goal
  could be a crash, memory exhaustion, integer overflow, out-of-bounds read, or
  a report that misleads a consumer.
- **Concurrent file modification.** `hoard-gguf` memory-maps the file. Another
  process (or the same user) truncating or replacing the underlying file while
  it is mapped can make the process receive `SIGBUS` when it touches the lost
  pages.
- **A malicious manifest.** A hand-crafted manifest could try path traversal
  (`../`, absolute paths), duplicate or unsorted entries, digest strings that
  are not digests, broken sibling relations, or a source revision that is a
  branch. A consumer that only parses without validating would trust it.
- **Supply chain.** A malicious or vulnerable crates.io dependency, a typosquat,
  a yanked release, or a non-crates.io source would compromise the binaries.
- **PLANNED (P2): untrusted source responses.** A fetch command will receive
  HTTP responses from a source host. Threats: wrong content for a URL, redirects
  to unexpected hosts, truncated bodies, injected filenames, mutable refs
  (`main` moving between resolution and download), and servers that lie about
  sizes or content types.
- **PLANNED (P2): cache poisoning.** Once a content-addressed store exists, a
  corrupted or attacker-written entry must never be trusted without re-hashing,
  and a partially written file must never appear at a final path.

## Mitigations

### Bounds-checked parser and caps

- Every length read from the file is checked against the bytes that remain
  before it is used; every count is checked against a cap before allocating.
- All element-count and byte-size arithmetic uses checked operations.
- Metadata arrays are capped at 2^20 elements and 32 nesting levels, and files
  are capped at 2^20 metadata entries; tensor counts are capped at 2^20; tensor
  names and metadata keys have length limits. The nesting cap matters because
  parsed arrays are recursively owned: without it, dropping a crafted
  deeply-nested value would abort the process with a stack overflow.
- The inspect report samples arrays and never recurses into nested arrays, so
  report size is bounded by the number of entries, not by declared element
  counts.
- Tensor overlap detection sorts tensor offsets (`O(n log n)`) rather than
  comparing every pair.

### One audited unsafe operation

The whole workspace denies `unsafe_code`; the single exception is
`memmap2::Mmap::map` in `crates/hoard-gguf/src/file.rs`, under a documented
`#[allow(unsafe_code)]` with a SAFETY comment. The mapping is read-only and lives
as long as the `GgufFile`; all parsing works on a borrowed `&[u8]`.

### Fail-closed validation

- Parsing either produces a validated `Gguf` or a typed error. There is no
  "best effort" mode and warnings never suppress errors that affect structure.
- Manifest validation is separate from parsing, and the CLI's `manifest
  validate`, `manifest hash`, and every consumer of `Manifest` validate before
  using the data. `manifest hash` refuses to hash an invalid manifest.
- `manifest fmt --check` fails when the committed bytes are not canonical, so a
  repository cannot silently drift from its documented identity.
- The planned transport (P2) must follow the same policy: a digest or size
  mismatch rejects the file, and there is no trust-on-first-use cache.

### Supply chain

- `Cargo.lock` is committed and CI builds with a pinned Rust toolchain
  (`rust-toolchain.toml`, and `dtolnay/rust-toolchain@1.99.1`).
- `cargo-deny` runs in CI with advisories, bans, licenses, and sources checks
  ([`deny.toml`](../deny.toml)). Dependencies must come from crates.io.
- `flake.lock` pins nixpkgs, and the Nix package resolves dependencies through
  `Cargo.lock` (`importCargoLock`), so the Nix build uses exactly the locked
  crates, each verified against its lockfile checksum.
- Release artefacts must be built from a clean checkout in CI. Signing and
  provenance attestations are not implemented in 0.1.0 (see below).

### Residual risk: `SIGBUS` from concurrent truncation

Memory mapping cannot be made robust against another process truncating the
file. If the file is truncated while it is mapped, touching the removed pages
raises `SIGBUS` and the process is terminated; there is no portable way to catch
and recover. Mitigations:

- hoard never writes to model files and holds the mapping only while inspecting.
- The limitation is documented on `GgufFile` and here, so callers can avoid
  inspecting files that are being written concurrently.
- Hashing (`hoard hash`) streams the file with `read` instead of mapping it, so
  a shrinking file cannot fault the process; it hashes the bytes that are
  actually readable. The printed `size` comes from a stat taken before hashing,
  so it can disagree with the hashed bytes if another process truncates the
  file concurrently.

This is a denial-of-service risk only; it cannot corrupt output that was
already printed.

## Out of scope

- **Publisher trust.** A manifest hash proves two manifests are identical, not
  that the publisher is honest. A malicious Hugging Face repository can publish
  files whose `sha256` and `blake3` match whatever it wants.
- **Model safety.** hoard does not decode tensor values and does not execute
  model code; it cannot tell whether a model's content is safe or correct.
  Vulnerabilities in `llama.cpp` or in a model loader that later consumes the
  bytes are outside hoard.
- **Signing and attestations.** Manifest v1 has no signature, no
  `Signed-off-by`, no timestamp, and no provenance attestation. CI does not
  attest releases in 0.1.0.
- **Same-user attackers.** An attacker with the same OS privileges as the
  running process can read and replace the files being inspected, change
  binaries, or attach to the process. `SIGBUS` is one symptom; the whole local
  threat model is out of scope.
- **Filesystem I/O failures.** A network filesystem that returns I/O errors on
  mapped pages can also fault the process; this is treated like truncation.
- **Traffic analysis / network metadata.** 0.1.0 makes no network requests.
- **Terminal escape injection.** Error and warning messages can contain strings
  copied from the input (metadata keys, tensor names, file paths). Do not render
  hoard output in a terminal or log viewer that interprets escape sequences
  without escaping it first. The report's JSON strings are escaped normally, but
  the human-readable output is meant for humans.

## Post-0.1.0 threats and planned mitigations

| Threat | Plan |
| --- | --- |
| Source host serves different bytes | Verify `sha256` and `blake3` before the file is used; fail closed. |
| Mutable branch resolution | Resolve a revision to a commit sha once, record it, download from the pinned sha URL. |
| MITM / redirect to unexpected host | HTTPS with system CA store, no plain HTTP, bounded redirects, host allow-list. |
| Truncated or partial download | Size check against the manifest, atomic temp-file rename into the CAS. |
| Cache poisoning | Re-hash on read; never trust a filename; write atomically and fsync. |
| Compromised dependency | Keep the tree small, keep advisories and sources checks green, review lockfile diffs. |

## Operational guidance

- Run `hoard manifest validate` and `hoard manifest hash` on manifests you did
  not write, and compare the hash to the value you expected from another channel.
- Check the `warnings` array before trusting an inspect report for model
  integrity decisions; overlapping tensors and unknown quantization types are
  accepted on purpose and reported there.
- Do not truncate or rewrite a model file while any process is inspecting it.
- Run `cargo deny check` before publishing or tagging a release; CI does it on
  every pull request.
- Treat manifests as code: review changes to `source.revision`, `sha256`,
  `blake3`, and `siblings` like you review dependency changes.
