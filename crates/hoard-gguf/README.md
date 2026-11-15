# hoard-gguf

Strict, zero-copy [GGUF](https://github.com/ggerganov/llama.cpp/blob/master/docs/gguf.md)
parsing and inspection for the hoard artifact pipeline.

`hoard-gguf` reads llama.cpp model files in GGUF version 2 and 3, little-endian,
validates every structure before it reports anything, and renders a versioned
JSON inspect report. Corrupted input produces a typed error. Unusual but
readable files produce stable warning codes.

## What it does

The parser takes a memory-mapped file, or any byte slice, and reads the header,
the metadata table, and the tensor infos. Keys, string values, and tensor names
point into the mapped bytes, so inspecting a large model does not copy its
contents.

Parsing checks the magic, version, byte order, counts, truncation, metadata
value types, quantization block geometry, duplicate tensor names, and tensor
bounds. It computes tensor sizes from the GGML quantization table and accounts
for alignment padding and file overhead.

A file that parses can still report warnings. The warning set covers unknown
quantization ids, unaligned or overlapping tensors, non-zero padding,
incomplete split metadata, and unusual metadata names. Warnings never change the
exit code or reject a file.

The inspect report is the `schema: 1` JSON document described in
[`docs/inspect.md`](../../docs/inspect.md). Its JSON Schema is generated from
the report types and committed as `schemas/inspect-v1.schema.json`.

## Usage

```rust
use hoard_gguf::{Gguf, GgufFile, inspect};

let file = GgufFile::open("model.gguf")?;
let gguf = Gguf::parse(file.bytes())?;

println!(
    "{} tensors, {} metadata entries",
    gguf.tensors().len(),
    gguf.metadata().len()
);
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
| `cli` | yes | Builds the `hoard-gguf` binary. The library itself has no CLI dependencies. |
| `testing` | no | Exposes the deterministic `fixture` builder used to regenerate the committed synthetic GGUFs and to build malformed files in tests. |

## Tests

```sh
cargo test -p hoard-gguf --all-features
```

The suite sweeps every truncation point of a synthetic fixture, mutates
malformed files, feeds arbitrary bytes through the parser, and checks the real
corpus in `fixtures/real/` when those files are present.

## Documentation

- [`docs/inspect.md`](../../docs/inspect.md): the frozen inspect JSON contract.
- [`docs/usage.md`](../../docs/usage.md): CLI usage.
- `schemas/inspect-v1.schema.json`: JSON Schema for the inspect report.

## Licence

Apache-2.0. See [LICENCE](../../LICENCE).
