# hoard-gguf

Strict, zero-copy [GGUF](https://github.com/ggerganov/llama.cpp/blob/master/docs/gguf.md)
parsing and inspection for the `hoard` artifact pipeline.

`hoard-gguf` reads llama.cpp model files (GGUF v2 and v3, little-endian only),
validates every structure up front, and renders a frozen, versioned JSON
inspect report. Adversarial input produces a typed error instead of a panic, and
unusual-but-readable files produce stable warning codes instead of failing.

## What it does

- Parses the header, metadata, and tensor infos of a GGUF file from a
  memory-mapped file (or any byte slice) without copying strings.
- Checks magic, version, byte order, counts, truncation, metadata value types,
  quantization block geometry, duplicate tensor names, and tensor bounds.
- Computes per-tensor sizes from the GGML quantization table and accounts for
  file overhead and alignment padding.
- Emits non-fatal warnings for unknown quantization ids, unaligned or
  overlapping tensors, non-zero padding, incomplete split metadata, and
  metadata naming oddities.
- Produces the `inspect` JSON document (`schema` 1) and its JSON Schema.

## Usage

```rust
use hoard_gguf::{Gguf, GgufFile, inspect};

let file = GgufFile::open("model.gguf")?;
let gguf = Gguf::parse(file.bytes())?;

println!("{} tensors, {} metadata entries", gguf.tensors().len(), gguf.metadata().len());
for warning in gguf.warnings() {
    eprintln!("warning: {warning}");
}

let report = inspect(&gguf, Some(std::path::Path::new("model.gguf")));
println!("{}", serde_json::to_string_pretty(&report)?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Features

| Feature | Default | Effect |
| --- | --- | --- |
| `cli` | yes | Builds the `hoard-gguf` binary (the library itself has no CLI dependencies). |
| `testing` | no | Exposes the deterministic `fixture` builder used to regenerate the committed synthetic GGUFs and to construct malformed files in tests. |

## Documentation

- [`docs/inspect.md`](../../docs/inspect.md) -- the frozen `inspect` JSON contract.
- [`docs/usage.md`](../../docs/usage.md) -- CLI usage.
- `schemas/inspect-v1.schema.json` -- JSON Schema for the inspect report.

## Tests

```sh
cargo test -p hoard-gguf --all-features
```

The suite includes byte-level truncation checks over a synthetic fixture,
malformed-file mutations, a property test that arbitrary bytes never panic, and
real-corpus checks against `fixtures/real/` when those files are present.

## Licence

Apache-2.0. See [LICENCE](../../LICENCE).
