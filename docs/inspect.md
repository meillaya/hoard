# Inspect report

`hoard-gguf inspect <path> --json` prints one JSON document describing a GGUF
file. The document is generated from the `hoard_gguf::InspectReport` Rust types
with `schemars`, and its shape is frozen at `schema: 1`. The machine-readable
contract is committed at
[`schemas/inspect-v1.schema.json`](../schemas/inspect-v1.schema.json).

The report is a *view*, not a decoder: it records tensor metadata and byte
sizes, never tensor values.

## Parse errors

A structurally invalid file never produces a report: `hoard-gguf inspect` exits
1 and prints `error: ...` to stderr. Hard errors include a bad magic, an
unsupported or big-endian version, truncation, invalid UTF-8 in strings, an
unknown or wrongly-typed metadata value (including a `general.alignment` that
is not a `u32`), a boolean payload other than 0/1, a count past its cap, more
than four tensor dimensions, duplicate tensor names, a quantization block
mismatch, a tensor or data section past the end of the file, and a zero
alignment. Non-fatal oddities are reported in `warnings` instead; the full list
is below.

## Example

The committed minimal synthetic fixture
(`crates/hoard-gguf/tests/fixtures/synthetic-minimal.gguf`) reports:

```json
{
  "schema": 1,
  "file": {
    "path": "tests/fixtures/synthetic-minimal.gguf",
    "size": 32,
    "version": 3,
    "byte_order": "little",
    "alignment": 32,
    "data_start": 32
  },
  "model": {
    "architecture": null,
    "name": null,
    "quantization_version": null
  },
  "split": null,
  "metadata": {
    "count": 0,
    "entries": []
  },
  "tensors": {
    "count": 0,
    "total_bytes": 0,
    "bytes_known": true,
    "unknown_types": 0,
    "entries": []
  },
  "quant_mix": [],
  "memory": {
    "tensor_bytes": 0,
    "file_bytes": 32,
    "overhead_bytes": 32,
    "padding_bytes": 0
  },
  "warnings": [
    {
      "code": "missing-architecture",
      "message": "no \"general.architecture\" metadata entry"
    }
  ]
}
```

The human-readable output (`inspect` without `--json`) is derived from the same
report. It is not a stable machine interface.

## `file`

| Field | Type | Meaning |
| --- | --- | --- |
| `path` | string or null | Path as given on the command line, or `null` when a library caller inspects a buffer. |
| `size` | integer | File size in bytes. |
| `version` | integer | GGUF version, always `2` or `3` (other versions are rejected before a report exists). |
| `byte_order` | string | Always `"little"` in schema 1; big-endian files are rejected. |
| `alignment` | integer | Tensor data alignment from `general.alignment`, default `32`. |
| `data_start` | integer | Absolute offset where the tensor data section starts, aligned up from the end of the tensor infos. |

## `model`

Extracted from three well-known metadata keys. Missing or type-mismatched keys
become `null`; they do not fail the command.

| Field | Type | Source key |
| --- | --- | --- |
| `architecture` | string or null | `general.architecture` |
| `name` | string or null | `general.name` |
| `quantization_version` | integer or null | `general.quantization_version` |

## `split`

`null` unless all of `split.no` (`u16`), `split.count` (`u16`), and
`split.tensors.count` (`i32`) are present and well-typed.

| Field | Type | Meaning |
| --- | --- | --- |
| `no` | integer | Zero-based index of this part. |
| `count` | integer | Total number of parts. |
| `tensors_count` | integer | Total tensor count across all parts. |

Degenerate split metadata never fails the report:

- Only some of the three keys present: `split` is `null` and the report carries
  a `split-metadata-incomplete` warning.
- All keys present but one has the wrong value type: `split` is `null` and
  there is **no** warning. The library's `Gguf::split()` returns a type error
  for this case, but the inspect report has no warning code for it.
- `split.no >= split.count`: `split` is `null` and the report carries a
  `split-index-out-of-range` warning.

## `metadata`

| Field | Type | Meaning |
| --- | --- | --- |
| `count` | integer | Number of metadata entries. |
| `entries` | array | One report per entry, in file order. |

A scalar entry:

```json
{ "key": "general.alignment", "type": "u32", "value": 64 }
```

An array entry (sampled, never recursed):

```json
{
  "key": "tokenizer.ggml.tokens",
  "type": "array",
  "array": {
    "element_type": "string",
    "length": 5,
    "sample": ["<unk>", "<s>", "</s>"],
    "sample_truncated": true
  }
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `key` | string | Metadata key. |
| `type` | string | Lower-case value type: `u8`, `i8`, `u16`, `i16`, `u32`, `i32`, `f32`, `bool`, `string`, `array`, `u64`, `i64`, `f64`. |
| `value` | any | Scalar value; absent for arrays. |
| `array.element_type` | string | Element type name; `"array"` for nested arrays. |
| `array.length` | integer | Number of elements. |
| `array.sample` | array | Up to three leading scalar elements; empty for nested arrays. |
| `array.sample_truncated` | boolean | Whether elements are omitted from `sample`. |

Array samples are bounded on purpose: a malicious file can declare millions of
elements, and the report must stay small. Nested arrays are not recursed.

## `tensors`

| Field | Type | Meaning |
| --- | --- | --- |
| `count` | integer | Number of tensor infos. |
| `total_bytes` | integer | Sum of the byte sizes of tensors with a known quantization type. |
| `bytes_known` | boolean | Whether every tensor has a known byte size. |
| `unknown_types` | integer | Number of tensors whose `ggml_type` id is unknown. |
| `entries` | array | One report per tensor, sorted by data offset then name. |

Each tensor entry:

| Field | Type | Meaning |
| --- | --- | --- |
| `name` | string | Tensor name. |
| `type` | string | Quantization type name (`F32`, `Q8_0`, `Q4_K`, ...) or `"unknown"`. |
| `type_id` | integer | Numeric `ggml_type` id. |
| `shape` | array of integers | Dimensions, innermost first. |
| `elements` | integer | Product of the dimensions; `1` for a scalar tensor. |
| `bytes` | integer or null | Data size in bytes; `null` when the quantization type is unknown. |
| `offset` | integer | Offset relative to `file.data_start`. |
| `file_offset` | integer | Absolute file offset (`data_start + offset`). |

## `quant_mix`

One entry per known quantization type, sorted by total bytes descending then by
type id. The list is empty when no tensor has a known type.

| Field | Type | Meaning |
| --- | --- | --- |
| `type` | string | Quantization type name. |
| `type_id` | integer | Numeric `ggml_type` id. |
| `tensors` | integer | Number of tensors with this type. |
| `elements` | integer | Sum of their element counts. |
| `bytes` | integer | Sum of their byte sizes. |
| `percent` | number | Share of known tensor bytes, two decimals. |

`percent` is computed from integer basis points, so it does not depend on
floating-point rounding. Percentages are of known tensor bytes only.

## `memory`

| Field | Type | Meaning |
| --- | --- | --- |
| `tensor_bytes` | integer | Sum of known tensor data sizes. |
| `file_bytes` | integer | Total file size. |
| `overhead_bytes` | integer | `file_bytes - tensor_bytes`. |
| `padding_bytes` | integer | Alignment gaps between tensors, including before the first tensor. |

`overhead_bytes` is not header size: it includes padding, tensor metadata, and
the bytes of unknown-type tensors. When `bytes_known` is false, both
`tensor_bytes` and `overhead_bytes` are approximations; use
`tensors.bytes_known` to tell.

## `warnings`

Non-fatal diagnostics, in parser emission order. Codes are stable kebab-case
strings; messages are human-readable and may change. Warning codes are not a
closed set: treat unknown codes as informational.

| Code | Meaning |
| --- | --- |
| `unusual-alignment` | `general.alignment` is not a multiple of 8. |
| `missing-architecture` | No `general.architecture` metadata entry. |
| `split-metadata-incomplete` | Only some of the `split.*` keys are present. |
| `split-index-out-of-range` | `split.no` is not smaller than `split.count`. |
| `metadata-key-not-ascii` | A metadata key contains non-ASCII bytes. |
| `metadata-key-too-long` | A metadata key is longer than 65535 bytes. |
| `duplicate-metadata-key` | Two metadata entries share a key. |
| `tensor-name-too-long` | A tensor name is longer than 64 bytes. |
| `empty-tensor-name` | A tensor name is empty. |
| `scalar-tensor` | A tensor has zero dimensions and one element. |
| `unknown-quantization-type` | A tensor uses an unknown `ggml_type` id. |
| `unaligned-tensor-offset` | A tensor offset is not a multiple of the alignment. |
| `overlapping-tensors` | Two tensor byte ranges overlap. |
| `non-zero-padding` | Alignment padding before the data section is not zero. |

Warnings never change the exit code. A file with warnings is still a valid
GGUF file for inspection.

## Formatting and precision

- Pretty JSON uses two spaces of indentation and a trailing newline.
- 64-bit counts and sizes are written as JSON integers. JSON numbers are
  IEEE-754 doubles, so a parser that decodes every number into a `double` loses
  precision above 2^53. `serde_json` (Rust), `jq`, and Python's `json` keep the
  integers exact; a JavaScript `JSON.parse` consumer should treat very large
  values as strings or use a big-integer parser.
- `f32`/`f64` metadata values are written as JSON numbers. NaN and infinity are
  not representable in JSON and are written as `null`.

## Schema freeze

`schemas/inspect-v1.schema.json` is generated from the Rust types and committed.
A unit test compares the committed file with the generated schema, so the
schema cannot silently drift:

```sh
UPDATE_SCHEMAS=1 CARGO_HOME=$PWD/.cargo-home cargo test -p hoard-gguf --lib schema
```

Regenerate only when the Rust types change on purpose, and commit the schema
together with the report changes.

Compatibility rules:

- Bumping `schema` is required for removed fields, renamed fields, changed
  types, changed units, or changed ordering guarantees.
- Adding an optional field does not require a bump, but consumers must ignore
  unknown fields. Adding a warning code does not require a bump.
- The human-readable output and warning message strings are outside the frozen
  contract.
