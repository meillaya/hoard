# Fetch and verify artifacts

`hoard fetch` downloads a pinned Hugging Face file set, verifies it, and writes a
canonical manifest. `hoard verify` re-checks those bytes offline. The same
manifest drives the Nix helpers that turn each file into a fixed-output
derivation.

This page is the how-to and the workflow reference. Every command and flag also
appears in [usage](usage.md).

## Before you start

- Build the binary, or use the repository shell. The commands below assume
  `hoard` is on `PATH`.

  ```sh
  devenv shell
  cargo build --workspace --release
  export PATH="$PWD/target/release:$PATH"
  ```

- Plan for disk space. The content-addressed store keeps one copy per distinct
  file, and `--out` keeps a second copy where you ask for it. The default store
  root is `$XDG_CACHE_HOME/hoard/store`, then `$HOME/.cache/hoard/store`;
  `HOARD_STORE` overrides both.
- Fetch needs network access on the first run. Later runs can use `--offline`
  against the store.
- A gated or private repository needs a token in `HF_TOKEN` or
  `HUGGING_FACE_HUB_TOKEN`. Public repositories work anonymously.
- `hoard store` needs `nix-store` on `PATH`. `hoard push` needs `nix` on `PATH`.
- A manifest pins exact bytes. Review `source.revision`, `sha256`, `blake3`, and
  `siblings` before you trust a manifest you did not write.

## Fetch one file from a spec

1. Choose a repository, a revision, and a repository path. The spec syntax is
   `hf:owner/name[@rev]`; without `@rev`, hoard resolves `main` to a commit.

   ```text
   hf:ggml-org/models@499bc8821c6b12b4e53c5bffcb21ec206f212d81
   tinyllamas/stories260K.gguf
   ```

2. Run the fetch. `--manifest-out` is required with a spec, and `--out`
   materializes a verified copy for inspection.

   ```console
   $ hoard fetch hf:ggml-org/models@499bc8821c6b12b4e53c5bffcb21ec206f212d81 \
       --file tinyllamas/stories260K.gguf --manifest-out stories260K.json --out artifacts/
   fetched tinyllamas/stories260K.gguf (1185376 bytes, Xet, 0.76s)
   wrote stories260K.json (blake3:56bbfeba5585a228ce8ac25e41d2fba1b16f80e837f93257b61d8fda11bb2421)
   ```

3. Verify the result against a fresh directory copy.

   ```console
   $ hoard verify --manifest stories260K.json --dir artifacts/
   verified 1 file(s)
   ```

Expected end state: the CAS holds the verified blob, `stories260K.json` is
canonical manifest v1, and `artifacts/tinyllamas/stories260K.gguf` is readable
at the same path. The manifest records the source `sha256` from the repository
tree and the `blake3` hoard computed from the downloaded bytes.

## Fetch a split set from a manifest

The repository example `examples/nix/models/stories15M-q8_0.json` pins three
weights shards from one split model. Each entry records the shared group
`q8_0` and lists the other two shards as `siblings`.

1. Fetch every file in the manifest. With no `--file`, the default is every
   entry.

   ```console
   $ hoard fetch --manifest examples/nix/models/stories15M-q8_0.json --out split-out
   fetched tinyllamas/split/stories15M-q8_0-00001-of-00003.gguf (10517152 bytes, Xet, 1.33s)
   fetched tinyllamas/split/stories15M-q8_0-00002-of-00003.gguf (10381216 bytes, Xet, 1.05s)
   fetched tinyllamas/split/stories15M-q8_0-00003-of-00003.gguf (5773312 bytes, Xet, 0.96s)
   ```

2. Check the store entries.

   ```console
   $ hoard verify --manifest examples/nix/models/stories15M-q8_0.json --store
   verified 3 file(s)
   ```

Expected end state: three blobs in the CAS and the full split tree under
`split-out/`. A manifest that names only some parts of a split set fails
manifest validation, so hoard never records a partial sibling relation.

To build a new manifest for a split model from a spec, pass every part with
`--file` and the same `--group`. Hoard detects the `-NNNNN-of-NNNNN.gguf`
suffix, requires the complete set, and writes sorted symmetric `siblings`.

## Fetch from the cache and offline

`hoard fetch --manifest` reads every selected file from the CAS first and
downloads only the missing ones. `--offline` stops before any network call and
fails listing the missing paths.

```console
$ hoard fetch --manifest stories260K.json --offline
$ echo $?
0

$ HOARD_STORE=/tmp/empty hoard fetch --manifest stories260K.json --offline
error: missing from the CAS and --offline is set: ["tinyllamas/stories260K.gguf"]
$ echo $?
1
```

`--offline` verifies the CAS entries it finds, so a corrupt blob fails the
command rather than being skipped. Use `hoard verify --manifest <FILE> --store`
when you want only the verification step and no materialization.

## Choose a transport

`--transport auto` is the default. It uses Xet when the repository tree carries
a Xet hash for the file and plain HTTP otherwise. When the Xet transfer fails
for a transport reason, `auto` retries the file over HTTP; a digest or size
mismatch is never retried on another transport.

| Value | Behavior |
| --- | --- |
| `auto` | Xet when a Xet hash exists, HTTP fallback on transport failure. Default. |
| `xet` | Force Xet. Fails when the tree entry has no Xet hash. |
| `http` | Force plain HTTP with Range resume. |

HTTP downloads write `<destination>.part` and rename only after the SHA-256 and
size match. A truncated body leaves the part file so the next HTTP run can send
a Range request and continue; a digest mismatch removes it. Requests retry
3 times on 429, 5xx, connection, and timeout failures with exponential backoff
and jitter. Redirects follow at most 8 hops and stop at the Hugging Face hosts,
the Xet CAS host, or the configured endpoint host. Plain HTTP is accepted only
on the configured `HF_ENDPOINT` host, which keeps local mirrors and test servers
usable.

The Xet shard cache lives in `HOARD_XET_CACHE` when that variable is set. The
default is a temporary directory that is removed when the download finishes.

## Materialize files

`--out` copies verified blobs out of the CAS into a directory tree named by the
manifest paths, creating parent directories as needed. The write goes through a
temporary file and a rename, so a failure never leaves a partial destination.
Materialize refuses a symlink or a directory at the destination.

With `--manifest --file <one path>`, `--out` may name a file instead of an
existing directory:

```sh
hoard fetch --manifest stories260K.json \
  --file tinyllamas/stories260K.gguf --out single.gguf
```

A fetch from a spec always treats `--out` as a directory. Without `--out`, the
command only populates the CAS and writes the manifest.

## Verify a manifest

```text
hoard verify --manifest <FILE> [--dir <DIR> | --store]
```

`--store` is the default. It re-hashes each blob in the CAS and checks size,
SHA-256, and BLAKE3 against the manifest. `--dir` performs the same checks on
files under a directory instead.

```console
$ hoard verify --manifest stories260K.json --dir broken/
tinyllamas/stories260K.gguf: size mismatch: expected 1185376, found 0
error: 1 file(s) failed verification
$ echo $?
1
```

On success the command prints `verified <N> file(s)` to stderr and exits 0. On
failure it prints one stderr line per bad file, then exits 1.

## Nix consumer

The flake exports two helpers from `hoard.lib`:

| Helper | Arguments | Output |
| --- | --- | --- |
| `fetchArtifact` | `{ pkgs, hoard, manifest, path }` | One flat fixed-output store path for one manifest entry. |
| `fetchArtifactSet` | `{ pkgs, hoard, manifest }` | One derivation that symlinks every entry to its FOD. |

The consumer calls them with the hoard package for its own system:

```nix
hoard.lib.fetchArtifactSet {
  inherit pkgs;
  hoard = hoard.packages.${system}.default;
  manifest = ./models/stories15M-q8_0.json;
}
```

`examples/nix/flake.nix` is a complete consumer. It takes
`github:meillaya/hoard` as an input, follows that flake's nixpkgs, and builds the
example manifest as its default package:

```sh
cd examples/nix
nix build .#default \
  --override-input hoard "git+file:/path/to/hoard" \
  --no-write-lock-file --print-out-paths
```

The build prints one output path for the symlink farm:

```text
/nix/store/r4cc2c4nz7bil0cyzykfcj5c97dh9qn6-hoard-set-8a3a74042ae0
```

Each fixed-output derivation fetches one manifest entry with
`hoard fetch --manifest <manifest> --file <path> --out $out`. It sets `HOME`,
`XDG_CACHE_HOME`, and `HOARD_STORE` inside the build directory, forwards
`HF_TOKEN` as an impure environment variable, and relies on Nix to check the
declared hash before the bytes enter the store. The first build of a file needs
network access; later builds substitute the same store path.

### The flat-FOD decision

Nix fixed-output derivations accept `sha256` and `sha512`, and manifest v1
records the source `sha256`, so each file maps to one FOD without a new hash
format. The FODs use `outputHashMode = "flat"`: the store path holds the file
bytes, not a directory or a NAR wrapper. That gives three properties:

- The store path name is deterministic: `hoard-<first 12 characters of
  sha256>-<basename>`.
- `hoard store` can reproduce the same path with
  `nix-store --add-fixed sha256`, which is what the round-trip check compares.
- Nix caches and substitutes each file on its own. A set of 3 shards is 3
  FODs, not one recursive derivation that has to re-download when one file
  changes.

`fetchArtifactSet` assembles the set as a symlink farm over the per-file FODs.
The farm name is `hoard-set-<first 12 characters of the first entry's sha256>`.

### Pushing to a binary cache

```console
$ hoard push --manifest examples/nix/models/stories15M-q8_0.json --to file:///tmp/hoard-cache
copying 3 paths...
...
pushed 3 path(s) to file:///tmp/hoard-cache
```

`hoard push` verifies the CAS entries and runs `nix copy --to <STORE_URI>` on
the flat store paths. It hands signing to the cache: Nix signs with the secret
key configured for that cache, and a cache that requires signatures needs one.
Hoard holds no signing key and no cache credentials. The local-file round-trip
below uses `--option require-sigs false` because its temporary cache is
unsigned.

### Round-trip check

```sh
bash scripts/check-nix-roundtrip.sh
```

The script builds the example, copies its FODs to a temporary `file://` cache,
deletes the farm and the FODs, then rebuilds with `--offline`, the temporary
cache as substituter, and signature checks off. It compares the two output
paths and prints `round-trip ok: <path>` when they match.

## Failure behavior

| Situation | Result |
| --- | --- |
| Truncated HTTP body | Part file kept, next `http` or `auto` run resumes with a Range request. |
| SHA-256 or BLAKE3 mismatch | Part file removed, command exits 1, no manifest is written. |
| Source metadata disagrees with the manifest | `--manifest` fails before downloading that file. |
| Interrupted CAS write | The blob path appears only after a fsynced temporary file is renamed into place. |
| Interrupted materialization | Destination is replaced by an atomic rename; no partial file appears at the final path. |
| Corrupt CAS blob | `verify`, `store`, `push`, and `--out` reject the file with a size or digest mismatch. |
| `--offline` with a missing file | Command exits 1 and names every missing path. |
| Transient 429, 5xx, connection, or timeout | Retried 3 times with backoff, then reported as an error. |
| Redirect to a host outside the allow-list | Request fails; the response is not followed. |
| Fixture expires or moves | The resolve to a commit and the pinned revision in the manifest keep the expected bytes checkable. |

A failed fetch never writes a manifest. The manifest file appears only after
every selected file has verified, and the write itself is atomic.

## Current limits

- Hugging Face LFS/Xet files only. A file without a published LFS object id is
  rejected before any request is made.
- Public repositories are tested. The token path exists for gated and private
  repositories, but the test suite does not cover a private repository.
- Hugging Face is the only source. Manifest v1 records `kind: "huggingface"`,
  and there is no generic HTTPS, local directory, or S3 source.
- The CAS is whole-file, keyed by BLAKE3. Xet deduplicates transfer; the local
  store does not deduplicate partial content.
- Nix integration uses one flat FOD per file and a symlink farm for the set.
  There is no recursive NAR derivation.
- Fetch downloads files one at a time. There is no parallel download flag.
- Hoard does not sign or GC. `hoard push` delegates upload and signing to
  `nix copy`; the store has no prune command.
- There is no split writer or quantization decode. Fetch copies bytes; inspect
  reports metadata and sizes.
- `--offline` covers the CAS only. A manifest that is not on disk still has to
  come from somewhere you trust.

See [limits and non-goals](limitations.md) for the full list and
[security](security.md) for the threat model.
