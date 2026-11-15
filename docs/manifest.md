# Manifest v1

A manifest pins an artifact set to its source. It records the Hugging Face
repository and commit the files came from, and for each file a POSIX-relative
path, a role, a size, the source SHA-256, and the hoard BLAKE3. Manifests have
one canonical JSON encoding; the manifest hash (the artifact ID) is the BLAKE3
of that encoding.

`hoard-core` implements the format. The `hoard` CLI exposes it as
`hoard manifest validate|fmt|hash|schema`.

## Shape

```json
{
  "files": [
    {
      "blake3": "1b2a49c52e28947a42c0b93606e1cd4554dc97f046f37fb292fbaa3517acdcc0",
      "path": "tinyllamas/stories260K.gguf",
      "role": "weights",
      "sha256": "270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d",
      "size": 1185376
    }
  ],
  "schema": 1,
  "source": {
    "kind": "huggingface",
    "repo": "ggml-org/models",
    "repoType": "model",
    "revision": "499bc8821c6b12b4e53c5bffcb21ec206f212d81"
  }
}
```

Those are the canonical bytes of `fixtures/manifests/stories260k.json`.

## Fields

### Top level

| Field | Type | Rules |
| --- | --- | --- |
| `schema` | integer | Required; must be `1`. Unknown versions are rejected. |
| `source` | object | Required; see below. |
| `files` | array | Required; at least one entry, sorted by `path`, paths unique. |

Unknown keys anywhere are rejected (`deny_unknown_fields`).

### `source`

| Field | Type | Rules |
| --- | --- | --- |
| `kind` | string | Required; only `"huggingface"` in v1. |
| `repo` | string | Required; Hugging Face `owner/name` slug: exactly one `/`, both parts non-empty, ASCII letters/digits/`.`/`_`/`-` only, no leading or trailing `-`/`.`, no `..`. |
| `repoType` | string | Required; `"model"`, `"dataset"`, or `"space"`. |
| `revision` | string | Required; the 40-character lowercase hexadecimal commit sha. Not a `sha256:` string. |

### `files[]`

| Field | Type | Rules |
| --- | --- | --- |
| `path` | string | Required; POSIX-relative path: non-empty, at most 1024 bytes, no leading `/`, no `\`, no NUL, no `.`/`..`/empty segments. |
| `role` | string | Required; `weights`, `mmproj`, `mtp`, `tokenizer`, `config`, or `other`. |
| `group` | string | Optional; `^[a-z0-9][a-z0-9._-]{0,63}$`. Only allowed on `weights`, `mmproj`, and `mtp` files. Shared by related shards. |
| `size` | integer | Required; file size in bytes, greater than zero. |
| `sha256` | string | Required; 64 lowercase hex characters, from the Hugging Face LFS oid. |
| `blake3` | string | Required; 64 lowercase hex characters, hoard's content-address digest. |
| `siblings` | array | Optional; non-empty when present. Only allowed on `weights`. Sorted, self-excluding paths of other `weights` files in the same group. |

Sibling relations tie together related shards of one artifact, for example
`part-1.gguf` and `part-2.gguf` of a split model. A manifest is valid only when
every referenced path exists in `files`, points at a `weights` file with the
same `group` (including both being absent), and lists the referencing path
back: the relation is symmetric.

`Manifest::from_json_bytes`/`from_path` only parse. All of the above is enforced
by `Manifest::validate`, which rejects unsorted files, so a parsed manifest is
only trustworthy after it validates.

## Canonical form

Canonical JSON is the identity of a manifest. The rules:

- object keys are sorted lexicographically at every level;
- indentation is two spaces, line endings are LF, and the document ends with
  exactly one newline;
- `files` is sorted by `path` (byte-wise, which for UTF-8 is code-point order);
- each `siblings` list is sorted the same way;
- optional keys are omitted when empty (`group`, `siblings`), never written as
  `null` or `[]` by the canonical writer.

`Manifest::canonical_bytes` implements this and canonicalises even a manifest
parsed from non-canonical JSON. `hoard manifest fmt --check` fails when a file
is not canonical. Object key order does not depend on field declaration order,
so reordering struct fields does not change the hash.

## Hash and artifact ID

```text
manifest_hash = blake3(canonical_bytes)
```

written as `blake3:<64 lowercase hex>`. The hash is a pure function of the
canonical manifest: two manifests that differ only in JSON formatting, key
order, or file array order hash the same; any semantic change (including a
different revision or digest) hashes differently. `Manifest::hash` returns
`Blake3Digest`, which displays that form. The CLI prints the artifact ID with
`hoard manifest hash <path>`.

File digests are separate: `blake3` identifies file content in the
content-addressed store, and `manifest_hash` identifies the manifest that lists
it.

## Trust model

- The manifest hash is meaningful after validation. Before `validate` accepts a
  manifest, the file is only JSON; validation is what rejects path traversal,
  duplicate or unsorted entries, fake digest formats, and broken sibling
  relations.
- `sha256` is source metadata: it comes from the Hugging Face LFS oid and
  carries only the source's trust. Before a manifest records `blake3`, hoard
  computes it from the downloaded bytes.
- A manifest describes a file set; verification recomputes digests (or checks
  the CAS) against the files on disk. The manifest carries no signature, and
  version 1 records no provenance beyond the source repository and commit.
- A manifest holds no timestamps or other non-deterministic fields, so
  regenerating the same input produces byte-identical canonical JSON and the
  same hash.

## Real fixture

`fixtures/manifests/stories260k.json` is the real example above: one
1,185,376-byte GGUF from `ggml-org/models` at revision
`499bc8821c6b12b4e53c5bffcb21ec206f212d81`, path
`tinyllamas/stories260K.gguf`. The BLAKE3 comes from the downloaded file
`fixtures/real/stories260K.gguf` (gitignored; `scripts/fetch-fixtures.sh`
fetches it). Tests re-hash that file when it is present and skip when it is
absent.

`fixtures/manifests/synthetic.json` uses fabricated digests (`0000…`, `aaaa…`)
and exists only to pin canonicalisation and hash-golden behaviour. Do not treat
it as real artifact metadata.

## JSON Schema

`schemas/manifest-v1.schema.json` is generated from the Rust types by
`schemars` and committed (`* text=auto eol=lf` applies). The schema covers JSON
shape, key sets, enums, and digest patterns. Cross-field rules (`files` sorted,
distinct paths, non-empty and symmetric `siblings`, role/group restrictions)
belong to `hoard manifest validate`, so schema-valid JSON can still fail
validation. After changing the manifest types, regenerate it:

```sh
UPDATE_SCHEMAS=1 CARGO_HOME=$PWD/.cargo-home cargo test -p hoard-core --test schema
```

Then run the full suite; `committed_schema_is_current` fails when the file is
stale:

```sh
CARGO_HOME=$PWD/.cargo-home cargo test -p hoard-core
```
