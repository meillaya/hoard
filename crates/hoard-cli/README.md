# hoard-cli

The `hoard` command line: file hashes and manifest checks for model artifact
sets.

Version 0.1.0 ships the commands the artifact layer builds on:

```text
hoard hash <file> [--json]             size, SHA-256, BLAKE3
hoard manifest validate <path>         schema and semantics
hoard manifest fmt <path> [--check]    canonical bytes and canonical-form gate
hoard manifest hash <path>             blake3:<hex> artifact ID
hoard manifest schema                  manifest version 1 JSON Schema
```

The crate links [`hoard-core`](../hoard-core) only. GGUF inspection lives in the
separate `hoard-gguf` crate and binary. Both binaries read files and print
reports; they make no network requests.

See the repository [README](../../README.md), [usage](../../docs/usage.md), and
the [manifest contract](../../docs/manifest.md).
