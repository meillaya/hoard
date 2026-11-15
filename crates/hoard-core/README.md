# hoard-core

Manifest version 1 types, canonical JSON, validation, streaming hashes, a
content-addressed store, and the manifest builder for the hoard artifact layer.

A manifest records where an artifact set came from, a Hugging Face repository at
a pinned commit, and the files that make up the set. Each file carries its
SHA-256 as published by the source and its BLAKE3 as computed by hoard. The
manifest has one canonical JSON encoding, and its BLAKE3 hash over those bytes is
the artifact ID.

```rust
use hoard_core::Manifest;

let json = std::fs::read("fixtures/manifests/stories260k.json").unwrap();
let manifest = Manifest::from_json_bytes(&json).unwrap();
manifest.validate().unwrap();

println!("{}", manifest.hash());
let canonical = manifest.canonical_bytes();
assert!(canonical.ends_with(b"\n"));
```

## API

| Item | Purpose |
| --- | --- |
| `Manifest`, `Source`, `FileEntry`, `SourceKind`, `RepoType`, `Role` | Manifest version 1 data model (`serde` and `schemars`). |
| `Manifest::from_json_bytes`, `Manifest::from_path` | Parse JSON. Parsing does not validate. |
| `Manifest::validate` | Enforce every version 1 rule: paths, hashes, source, sorting, siblings. |
| `Manifest::canonical_bytes` | Canonical UTF-8 JSON with one trailing newline. |
| `Manifest::hash` | `Blake3Digest` over the canonical bytes. |
| `hash::sha256_file`, `hash::blake3_file` | Streaming file hashes in 1 MiB chunks. |
| `Hash32`, `Blake3Digest` | Lowercase-hex digest types. `Blake3Digest` displays as `blake3:<hex>`. |
| `cas::Store`, `cas::Entry` | Content-addressed store: `put_file`, `verify`, `materialize` under `<root>/blobs`. |
| `cas::store_root_from_env` | Store root from `HOARD_STORE`, `XDG_CACHE_HOME`, or `HOME`. |
| `artifact::build_manifest` | Join a source listing, CAS entries, and a selection into a validated manifest. |
| `artifact::infer_role`, `artifact::split_members` | Role inference and split-set detection. |
| `artifact::write_canonical` | Write canonical manifest bytes atomically. |
| `manifest_schema` | JSON Schema for manifest version 1, generated with `schemars`. |
| `Error`, `Result` | Shared error type with path and field context. |

`manifest::SCHEMA_VERSION` is `1`. The format rules, canonical form, and trust
model are in [`docs/manifest.md`](../../docs/manifest.md).

## Canonical form

Object keys are sorted at every level, indentation is two spaces, line endings
are LF, and the document ends with one newline. `files` is sorted by `path`, and
each `siblings` list is sorted. Use `Manifest::canonical_bytes` instead of
formatting a manifest by hand.

## Content-addressed store

`cas::Store` keeps verified bytes under `<root>/blobs/<first two hex>/<64 hex>`,
named by their BLAKE3 digest, with in-progress writes under `<root>/tmp`.

- `Store::put_file` streams SHA-256 and BLAKE3 in one pass, checks the expected
  source SHA-256, and renames a fsynced temporary file into place. A failed put
  leaves no blob behind, and an existing blob that verifies is reused.
- `Store::verify` re-hashes a blob and checks its size, BLAKE3, and SHA-256; a
  poisoned store entry is rejected instead of used.
- `Store::materialize` verifies first, then copies the blob to a destination
  with mode 0644, creating parent directories with mode 0755. It refuses a
  symlink or a directory at the destination.
- `cas::store_root_from_env` returns `$HOARD_STORE`, then
  `$XDG_CACHE_HOME/hoard/store`, then `$HOME/.cache/hoard/store`.

## Manifest builder

`artifact::build_manifest` joins a source listing (`SourceFile`), the
content-addressed entries for the fetched bytes, and a `Selection`. It writes
`size` and `sha256` from the source, `blake3` from the CAS entry, infers roles
from the paths when the selection has no role override, writes the group on the
selected weights, and writes sorted symmetric siblings for complete split sets
such as `model-00001-of-00003.gguf`. The built manifest is validated before it
is returned, and `artifact::write_canonical` writes its canonical bytes
atomically.

## Development

```sh
CARGO_HOME=$PWD/.cargo-home cargo test -p hoard-core
```

The committed `schemas/manifest-v1.schema.json` is checked by a test. Regenerate
it after changing the manifest types:

```sh
UPDATE_SCHEMAS=1 CARGO_HOME=$PWD/.cargo-home cargo test -p hoard-core --test schema
```

`fixtures/manifests/stories260k.json` is the real fixture used by the CLI tests.
Its BLAKE3 is computed from `fixtures/real/stories260K.gguf`, which is
gitignored; fetch it with `scripts/fetch-fixtures.sh`.
`fixtures/manifests/synthetic.json` holds fabricated data for canonicalisation
and hash-golden tests.

## Licence

Apache-2.0, matching the workspace.
