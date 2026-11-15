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

The crates are not published to crates.io in 0.1.0. Both binaries are
read-only: they open files and print output. No command writes a model file,
edits a manifest in place, or makes a network request.

## Exit codes

Both binaries share one exit-code convention:

| Code | Meaning |
| --- | --- |
| 0 | Success. Warnings from `inspect` do not affect the exit code. |
| 1 | Command failure: unreadable file, malformed GGUF, invalid manifest, non-canonical manifest under `--check`, or I/O error. Errors are printed to stderr as `error: ...`. |
| 2 | Usage error: unknown command or option, missing required argument (clap). |

`--help`/`-h` prints help and `--version`/`-V` prints `<binary> <version>`.
Both binaries accept no other global options, and hoard reads no configuration
file or environment variable.

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

## Worked example

Verify a downloaded model and its manifest from scratch:

```sh
# 1. Download a model at a pinned Hugging Face revision (any tool; hoard does
#    not download in 0.1.0).
curl -L --fail -o stories260K.gguf \
  "https://huggingface.co/ggml-org/models/resolve/499bc8821c6b12b4e53c5bffcb21ec206f212d81/tinyllamas/stories260K.gguf"

# 2. Check what it is and whether it looks sane.
hoard-gguf inspect stories260K.gguf

# 3. Compute the source digest and the content address.
hoard hash stories260K.gguf --json

# 4. Validate and identify the manifest that pins the file set.
hoard manifest fmt manifests/stories260k.json --check
hoard manifest validate manifests/stories260k.json
hoard manifest hash manifests/stories260k.json
```

The real fixture manifest pins `tinyllamas/stories260K.gguf` from
`ggml-org/models` at revision `499bc8821c6b12b4e53c5bffcb21ec206f212d81`. Its
`sha256` is the Hugging Face LFS oid, and its `blake3` must equal the output of
`hoard hash` on the downloaded file. The committed example is
`fixtures/manifests/stories260k.json`.

## Limits of the command surface

The 0.1.0 command surface is offline and read-only. Both binaries read the
files you name and print output; neither downloads, modifies those files, keeps
a cache, or stores artifacts. Neither opens a network connection.

There is no GGUF writer, converter, split, merge, quantizer, or tensor decoder.
Manifests can be validated, formatted, hashed, and schema-printed; hoard does
not create one from a directory or from the network. `inspect` reports metadata
and byte sizes and never reads tensor values. See
[limits and non-goals](limitations.md) for the full list.
