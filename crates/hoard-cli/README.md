# hoard-cli

The `hoard` command line: artifact identities for large model files.

0.1.0 ships the commands the artifact layer builds on:

```text
hoard hash <file> [--json]             # size, SHA-256, BLAKE3
hoard manifest validate <path>         # schema and semantics
hoard manifest fmt <path> [--check]    # canonical bytes / canonical-form gate
hoard manifest hash <path>             # blake3:<hex> artifact ID
hoard manifest schema                  # manifest v1 JSON Schema
```

This crate only links [`hoard-core`](../hoard-core); GGUF inspection lives in
the separate `hoard-gguf` crate and binary. Both binaries are read-only and
offline.

See the repository [README](../../README.md), [usage](../../docs/usage.md), and
the [manifest contract](../../docs/manifest.md).
