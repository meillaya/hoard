# Usage

## Install and invoke

Development builds run inside the repository shell:

```sh
devenv shell
cargo build --workspace --release
./target/release/hoard --version
./target/release/hoard-gguf --version
```

A system or rustup Rust 1.99.1 with plain `cargo` is the secondary path.

```text
hoard 0.1.0
hoard-gguf 0.1.0
```

Or build with Nix; see [development](development.md):

```sh
nix build .#default
./result/bin/hoard --version
./result/bin/hoard-gguf --version
```

The 0.1.0 crates are published on crates.io. Install the binaries with:

```sh
cargo install hoard-artifact --locked
cargo install hoard-gguf --locked
```

The package is `hoard-artifact` because the `hoard` and `hoard-cli` names are
taken; the installed binary is still `hoard`. `hoard-gguf`, `hoard hash`, and
the `hoard manifest` commands are read-only and offline. `hoard fetch` is the
one command that uses the network: it downloads files into the CAS, verifies
both digests, and for a spec writes a canonical manifest. `hoard verify` and
`hoard store` read the CAS, and `hoard push` hands the CAS paths to `nix copy`.

## Exit codes

Both binaries share one exit-code convention:

| Code | Meaning |
| --- | --- |
| 0 | Success. Warnings from `inspect` do not affect the exit code. `fetch` prints progress to stderr and exits 0 after every selected file verifies (and, for a spec, after the manifest is written). |
| 1 | Command failure: unreadable file, malformed GGUF or manifest, digest or size mismatch, missing CAS entry, failed verification, failed `nix-store` or `nix copy`, or I/O error. Errors are printed to stderr as `error: ...`. |
| 2 | Usage error: unknown command or option, missing required argument, or conflicting arguments (clap). |

`--help`/`-h` prints help and `--version`/`-V` prints `<binary> <version>`.
There are no other global options and no configuration file. `hoard fetch`
reads its endpoint and token from the environment variables below.

## Environment variables

| Variable | Read by | Meaning |
| --- | --- | --- |
| `HOARD_STORE` | `hoard` | Content-addressed store root. Overrides the default. |
| `XDG_CACHE_HOME` | `hoard` | Base for the default store root: `$XDG_CACHE_HOME/hoard/store`. |
| `HOME` | `hoard` | Fallback base for the default store root: `$HOME/.cache/hoard/store`. |
| `HF_ENDPOINT` | `hoard fetch` | Hugging Face endpoint, default `https://huggingface.co`. Point it at a mirror or a test server. |
| `HF_TOKEN` | `hoard fetch` | Bearer token for gated or private repositories. Takes precedence over `HUGGING_FACE_HUB_TOKEN`. |
| `HUGGING_FACE_HUB_TOKEN` | `hoard fetch` | Fallback token variable. |
| `HOARD_XET_CACHE` | `hoard fetch` | Xet shard cache directory. The default is a temporary directory removed after the download. |

The token goes only into request headers. The CLI has no token flag, so a token
stays out of the process argument list, and error messages never include it.
`HF_TOKEN` is also the one variable the Nix `fetchArtifact` helper forwards to
its build sandbox.

## `hoard-gguf inspect`

```text
hoard-gguf inspect [--json] [--tensors] <PATH>
```

Parses and validates a GGUF file and prints a report.

| Option | Meaning |
| --- | --- |
| `--json` | Print the frozen inspect JSON (`schema: 1`, two-space indent, trailing newline) instead of the human report. |
| `--tensors` | Add the full tensor table to the human report. |

The human report covers the file, quantization mix, metadata, the tensor table
when requested, and memory accounting. The command prints warnings to stderr as
`warning: <code>: <message>` and does not change the exit code. `--json` is the
stable machine interface; the human output is not. See
[the inspect report](inspect.md) for every field.

```console
$ hoard-gguf inspect fixtures/real/stories260K.gguf
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

metadata (19 keys)
  tokenizer.ggml.tokens                    array              array<string>[512]  "<unk>", "<s>", "</s>", ...
  tokenizer.ggml.scores                    array              array<f32>[512]  0.0, 0.0, 0.0, ...
  tokenizer.ggml.token_type                array              array<i32>[512]  2, 3, 3, ...
  tokenizer.ggml.model                     string             "llama"
  general.architecture                     string             "llama"
  general.name                             string             "llama"
  ...
```

`--json` is meant to be piped; each command below answers one question:

```sh
# How many tensors, and how big are they?
hoard-gguf inspect model.gguf --json | jq '.tensors.count, .tensors.total_bytes'

# Which quantization types are present and in what share?
hoard-gguf inspect model.gguf --json \
  | jq -r '.quant_mix[] | "\(.type)\t\(.percent)%\t\(.bytes) bytes"'

# Just the warnings as code/message pairs.
hoard-gguf inspect model.gguf --json | jq -r '.warnings[] | "\(.code): \(.message)"'

# Tensor table with shapes and absolute file offsets.
hoard-gguf inspect model.gguf --json \
  | jq -r '.tensors.entries[] | "\(.name)\t\(.type)\t\(.shape | join("x"))\t@\(.file_offset)"'
```

The JSON output always includes the tensor table. The human report includes it
only with `--tensors`.

### Errors

A file that is not GGUF, a truncated file, a big-endian file, or a file that
violates the caps in [limits and non-goals](limitations.md) fails with exit 1.
The diagnostic names the path and the parser error:

```console
$ hoard-gguf inspect fixtures/real/stories260K-be.gguf
error: failed to parse fixtures/real/stories260K-be.gguf: big-endian GGUF is not supported (write little-endian)
$ echo $?
1
```

## `hoard-gguf schema`

```text
hoard-gguf schema
```

Prints the inspect JSON Schema (`schema: 1`) to stdout. The output is the
committed `schemas/inspect-v1.schema.json`, so
`hoard-gguf schema | diff - schemas/inspect-v1.schema.json` exits 0.

## `hoard hash`

```text
hoard hash [--json] <FILE>
```

Computes two digests of a regular file, streaming it in 1 MiB chunks:

| Digest | Role |
| --- | --- |
| `sha256` | Source metadata: matches the Hugging Face LFS oid for a downloaded file. |
| `blake3` | Content address: the identity hoard records in manifests and uses for `blake3:` artifact IDs. |

```console
$ hoard hash fixtures/real/stories260K.gguf
fixtures/real/stories260K.gguf
  size    1185376
  sha256  270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d
  blake3  1b2a49c52e28947a42c0b93606e1cd4554dc97f046f37fb292fbaa3517acdcc0
```

`--json` prints an object with sorted keys and integer `size`:

```json
{
  "blake3": "1b2a49c52e28947a42c0b93606e1cd4554dc97f046f37fb292fbaa3517acdcc0",
  "file": "fixtures/real/stories260K.gguf",
  "schema": 1,
  "sha256": "270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d",
  "size": 1185376
}
```

`hoard hash` accepts regular files only. A directory or a missing path fails
with exit 1.

## `hoard manifest`

The manifest subcommands operate on a manifest v1 JSON file and validate it
before doing anything else. [manifest](manifest.md) specifies the format.

### `hoard manifest validate`

```text
hoard manifest validate <PATH>
```

Runs every v1 rule: schema version, source shape, repository slug, revision
format, file paths (no traversal, no backslashes, no NUL), roles, groups,
positive sizes, lowercase hex digests, sorting, duplicates, and symmetric
sibling relations. A valid manifest prints nothing and exits 0. An invalid one
prints the first error context and exits 1.

```console
$ hoard manifest validate fixtures/manifests/stories260k.json
$ echo $?
0
```

### `hoard manifest fmt`

```text
hoard manifest fmt [--check] <PATH>
```

Prints the canonical JSON encoding to stdout and leaves the input file alone.
To rewrite a file, redirect deliberately:

```sh
hoard manifest fmt manifest.json > manifest.json.tmp && mv manifest.json.tmp manifest.json
```

`--check` writes nothing and exits 1 with a message when the file is not
already canonical, which makes it usable as a CI gate:

```console
$ hoard manifest fmt fixtures/manifests/stories260k.json --check
$ echo $?
0

$ hoard manifest fmt /tmp/noncanonical.json --check
error: /tmp/noncanonical.json is not canonical; run `hoard manifest fmt /tmp/noncanonical.json` and redirect the output
$ echo $?
1
```

The check catches formatting, key ordering, file ordering, sibling ordering, and
a missing trailing newline. It does not rewrite the manifest.

### `hoard manifest hash`

```text
hoard manifest hash <PATH>
```

Prints the artifact ID, the BLAKE3 of the manifest's canonical bytes, with a
`blake3:` prefix:

```console
$ hoard manifest hash fixtures/manifests/stories260k.json
blake3:56bbfeba5585a228ce8ac25e41d2fba1b16f80e837f93257b61d8fda11bb2421
```

The command validates the manifest before hashing, so the printed ID always
satisfies the v1 rules.

### `hoard manifest schema`

```text
hoard manifest schema
```

Prints the manifest JSON Schema (`schema: 1`) to stdout. The output matches the
committed `schemas/manifest-v1.schema.json`.

## `hoard fetch`

```text
hoard fetch <SPEC> --file <PATH>... --manifest-out <FILE> [--group <GROUP>]
    [--repo-type model|dataset|space] [--role ROLE] [--out <DIR>]
    [--transport auto|xet|http]

hoard fetch --manifest <FILE> [--file <PATH>...] [--out <DIR>]
    [--transport auto|xet|http] [--offline]
```

The first form resolves a repository revision, downloads the named files,
verifies them, stores them in the content-addressed store, and writes a new
canonical manifest. The second form fetches the files listed in an existing
manifest at the manifest's pinned revision and verifies them against it.

`SPEC` has the form `hf:owner/name[@rev]`. Without `@rev`, the command resolves
`main`. The client resolves the revision to a full 40-character commit sha
before it lists files, so a generated manifest pins a commit.

| Option | Meaning |
| --- | --- |
| `SPEC` | Repository and revision as `hf:owner/name[@rev]`. Conflicts with `--manifest`. |
| `--manifest <FILE>` | Fetch the files listed in an existing manifest instead of a spec. Conflicts with `SPEC`. |
| `--file <PATH>` | Repository file to fetch. Repeat for several files. Required with a spec; with `--manifest`, the default is every file in the manifest. |
| `--group <GROUP>` | Group written to the weights entries of a new manifest. Ignored with `--manifest`. |
| `--repo-type <REPO_TYPE>` | Repository kind for a spec: `model` (default), `dataset`, or `space`. |
| `--role <ROLE>` | Role override for every selected file in a new manifest: `weights`, `mmproj`, `mtp`, `tokenizer`, `config`, or `other`. Ignored with `--manifest`. |
| `--manifest-out <FILE>` | Path for the generated manifest. Required with a spec; rejected with `--manifest`. |
| `--out <DIR>` | Materialize verified files under this directory. With `--manifest --file` selecting one file, the path can name a file instead. |
| `--transport <TRANSPORT>` | Download transport: `auto` (default), `xet`, or `http`. |
| `--offline` | Read only the CAS and fail when a file is missing. Valid only with `--manifest`. |

Fetch from a spec:

- The command lists the repository tree at the resolved commit. Every `--file`
  path must appear in the tree.
- Each selected file must carry a published SHA-256 LFS object id. A file
  without one is rejected before any request is made.
- `auto` prefers Xet when the tree entry carries a Xet hash and falls back to
  plain HTTP when the Xet transfer fails. A digest or size mismatch never falls
  back or retries; the bytes are known to be wrong.
- The command checks the published SHA-256 and size, computes BLAKE3 while it
  stores the file in the CAS, and writes the manifest only after every selected
  file verifies.
- Without `--role`, the role comes from the base name: `mmproj*` is `mmproj`,
  `mtp*` is `mtp`, `tokenizer*` and `vocab*` are `tokenizer`, `config.json` is
  `config`, `*.gguf` is `weights`, and everything else is `other`.
- When the selected weights form a split set named `-NNNNN-of-NNNNN.gguf`, the
  manifest records symmetric `siblings` and, with `--group`, the shared group. A
  partial split set fails the command.

```console
$ hoard fetch hf:ggml-org/models@499bc8821c6b12b4e53c5bffcb21ec206f212d81 \
    --file tinyllamas/stories260K.gguf --manifest-out stories260K.json --out artifacts/
fetched tinyllamas/stories260K.gguf (1185376 bytes, Xet, 0.76s)
wrote stories260K.json (blake3:56bbfeba5585a228ce8ac25e41d2fba1b16f80e837f93257b61d8fda11bb2421)
```

Progress and the manifest path go to stderr; stdout stays empty. Add
`--transport http` to force plain HTTP, or `--transport xet` to require Xet.

Fetch from a manifest:

`--manifest` uses the repository and pinned revision recorded in the manifest
and downloads only files missing from the CAS. It checks the remote size and
SHA-256 against the manifest first and fails when they disagree. A downloaded
file is stored by BLAKE3 and compared with the manifest before it is accepted.
Files already in the CAS are re-hashed and reused. `--offline` performs the CAS
checks and skips every network call.

```console
$ hoard fetch --manifest stories260K.json --offline
$ echo $?
0

$ HOARD_STORE=/tmp/empty hoard fetch --manifest stories260K.json --offline
error: missing from the CAS and --offline is set: ["tinyllamas/stories260K.gguf"]
$ echo $?
1
```

### Errors

| Error | Cause |
| --- | --- |
| `fetch from a spec needs at least one --file` | A spec form was used with no `--file`. |
| `--manifest-out is required when fetching from a spec` | A spec form was used without `--manifest-out`. |
| `--offline needs --manifest; a spec has to be resolved online` | `--offline` was combined with a spec. |
| `"<path>" is not in the repository tree` | A `--file` path is missing from the repository at the resolved commit. |
| `<file> has no LFS sha256` | The tree entry has no LFS object id, so manifest v1 cannot pin it. |
| `source metadata for <path> does not match the manifest` | The remote size or SHA-256 differs from the manifest. |
| `blake3 mismatch for <path>` | Downloaded bytes hash to a different BLAKE3 than the manifest records. |
| `missing from the CAS and --offline is set` | `--offline` found a manifest file absent from the store. |

Errors go to stderr as `error: ...` and the command exits 1. Conflicting
arguments such as `SPEC` with `--manifest` are usage errors, so clap exits 2.

## `hoard verify`

```text
hoard verify --manifest <FILE> [--dir <DIR> | --store]
```

Checks every file in a manifest against a directory or the content-addressed
store. `--store` is the default and re-hashes each CAS blob. `--dir` joins each
manifest path to the directory and checks size, SHA-256, and BLAKE3.

The command prints one stderr line per failing file and then exits 1. On success
it prints `verified <N> file(s)` to stderr and exits 0.

```console
$ hoard verify --manifest stories260K.json --dir artifacts/
verified 1 file(s)
$ echo $?
0

$ hoard verify --manifest stories260K.json --dir broken/
tinyllamas/stories260K.gguf: size mismatch: expected 1185376, found 0
error: 1 file(s) failed verification
$ echo $?
1
```

## `hoard store`

```text
hoard store --manifest <FILE>
```

Verifies every CAS entry, then adds each file to the Nix store as a flat
fixed-output path with `nix-store --add-fixed sha256`. The store path name is
`hoard-<first 12 characters of sha256>-<basename>`, the same path the
`lib.fetchArtifact` Nix helper produces. The command prints one
`<manifest path><TAB><nix store path>` line per file to stdout and requires
`nix-store` on `PATH`.

```console
$ hoard store --manifest examples/nix/models/stories15M-q8_0.json
tinyllamas/split/stories15M-q8_0-00001-of-00003.gguf	/nix/store/3ay96l18p4xl2ddjgmh890l70h25arhc-hoard-8a3a74042ae0-stories15M-q8_0-00001-of-00003.gguf
tinyllamas/split/stories15M-q8_0-00002-of-00003.gguf	/nix/store/x69yic63apbdbpln25326phk6q8c1x2q-hoard-4a70d35fdd27-stories15M-q8_0-00002-of-00003.gguf
tinyllamas/split/stories15M-q8_0-00003-of-00003.gguf	/nix/store/443y6k9hr978hjkn175fq1qg22q0197p-hoard-b2d72a3315df-stories15M-q8_0-00003-of-00003.gguf
```

## `hoard push`

```text
hoard push --manifest <FILE> --to <STORE_URI>
```

Verifies the CAS entries and runs `nix copy --to <STORE_URI>` on the flat store
paths. `hoard` passes no signing key: the destination cache's policy and signing
configuration decide whether a path is signed. `nix copy` prints its own
progress, and hoard prints `pushed <N> path(s) to <URI>` to stderr on success.
The command requires `nix` on `PATH`.

```console
$ hoard push --manifest examples/nix/models/stories15M-q8_0.json --to file:///tmp/hoard-cache
copying 3 paths...
...
pushed 3 path(s) to file:///tmp/hoard-cache
```

## Content-addressed store

`hoard fetch` writes verified bytes to a content-addressed store named by
BLAKE3. Blobs live at `<root>/blobs/<first two hex>/<64 hex>`, and in-progress
writes live under `<root>/tmp`. The store has no index and no lock file, so
concurrent writers are safe; each write renames a fsynced temporary file into
place.

The root is, in order: `HOARD_STORE`, `$XDG_CACHE_HOME/hoard/store`,
`$HOME/.cache/hoard/store`, then `.cache/hoard/store` when none of those
variables is set.

Storing a new blob reads the file twice. The first pass computes SHA-256 and
BLAKE3 and checks the expected source digest; the second pass copies the file
while re-hashing it and rejects the copy when the source changed. Commands that
read a blob (`verify`, `store`, `push`, and `--out` materialization) re-hash it
and reject a size, SHA-256, or BLAKE3 mismatch.

## Worked example

Fetch a model, check it, and identify the artifact set:

```sh
# 1. Fetch the file and write the manifest.
hoard fetch hf:ggml-org/models@499bc8821c6b12b4e53c5bffcb21ec206f212d81 \
  --file tinyllamas/stories260K.gguf --manifest-out stories260K.json --out artifacts/

# 2. Check what it is.
hoard-gguf inspect artifacts/tinyllamas/stories260K.gguf

# 3. Re-verify the fetched bytes and identify the manifest.
hoard verify --manifest stories260K.json --dir artifacts/
hoard manifest fmt stories260K.json --check
hoard manifest hash stories260K.json
```

The committed example `fixtures/manifests/stories260k.json` pins the same file
from `ggml-org/models` at revision
`499bc8821c6b12b4e53c5bffcb21ec206f212d81`. Its `sha256` is the Hugging Face
LFS oid, and its `blake3` matches the output of `hoard hash` on the downloaded
file. `hoard fetch` from a spec writes a manifest in the same format.

## Limits of the command surface

The command surface splits into an offline core and the network path:

- `hoard-gguf`, `hoard hash`, and `hoard manifest` read local files and print
  output. They do not write files or open network connections.
- `hoard fetch` is the only command that downloads. It writes the CAS, the
  manifest you name, and any `--out` files, and it is the only command that
  reads `HF_TOKEN`.
- `hoard verify` and `hoard store` read the CAS. `hoard store` runs
  `nix-store`; `hoard push` runs `nix copy` against the destination you name.
- Fetch supports Hugging Face LFS/Xet files only. A file without a published
  LFS SHA-256 is rejected.
- `manifest.source.kind` has one variant, so the source must be Hugging Face.
  There is no generic URL, local directory, or S3 source.
- The CAS stores whole files, not content-defined chunks. Xet deduplicates
  transfer; the local store does not deduplicate partial content.
- Nix integration uses one flat fixed-output derivation per file. There is no
  recursive NAR derivation.
- `hoard` holds no signing keys. `hoard push` delegates signing and upload to
  `nix copy`.
- Downloads run one file at a time. There is no store garbage collection or
  prune command.

There is no GGUF writer, converter, split, merge, quantizer, or tensor decoder.
`inspect` reports metadata and byte sizes and never reads tensor values. See
[limits and non-goals](limitations.md) for the full list.
