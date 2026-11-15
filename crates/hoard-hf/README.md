# hoard-hf

Hugging Face Hub resolution and Xet or HTTP transport for the hoard artifact
layer. The crate resolves a repository revision, lists the files at that commit,
and downloads one verified file at a time. Manifest validation and
content-addressed storage live in `hoard-core`.

The API is blocking. Call it from a normal thread, not from inside an async
runtime.

## What it does

`HubClient::resolve` turns a branch, tag, or commit into the 40 character commit
hash that pins the repository. `HubClient::list` reads the recursive tree and
keeps the files, with the LFS digest as `sha256` and the Xet content hash as
`xet_hash`. `HubClient::download` moves one file to a destination path.

Every download is checked against the SHA-256 digest from the tree and against
the expected size before it becomes visible at the destination. Bytes are
written to `<dest>.part` first. A killed process leaves a part file, never a
final path that looks complete. A digest mismatch removes the part file; a
truncated body keeps it so the next call can resume with a Range request.

## API

| Item | Purpose |
| --- | --- |
| `HubClient::new` | Build a client with an optional endpoint and token. The default endpoint is `https://huggingface.co`. |
| `client_from_env` | Build a client from `HF_ENDPOINT`, `HF_TOKEN`, or `HUGGING_FACE_HUB_TOKEN`. |
| `HubClient::resolve` | Resolve a revision to a `CommitSha`. |
| `HubClient::list` | List files at a commit, directories filtered out and sorted by path. |
| `HubClient::download` | Download one `RemoteFile` to a path and return `DownloadStats`. |
| `RepoSpec`, `RepoKind` | A repository reference: model, dataset, or space, plus `owner/name`. |
| `CommitSha` | A validated 40 character lowercase commit hash. Parses with `FromStr`. |
| `RemoteFile` | Repository path, size, optional SHA-256, optional Xet hash. |
| `Transport` | `Auto`, `Xet`, or `Http`. |
| `DownloadStats` | Verified byte count, transport actually used, wall time. |
| `Error` | Typed errors with path and status context. |

## Transports

`Transport::Xet` and `Transport::Http` force one transport. `Transport::Auto`
uses Xet when the file entry has a Xet hash and plain HTTP otherwise. When the
Xet transfer fails under `Auto`, the client falls back to HTTP and the returned
statistics report `Transport::Http`.

Files without a SHA-256 digest are rejected before any request is made. The
first manifest version requires the digest published by the Hub, so a file
without one cannot be pinned.

`Transport::Xet` fetches the xet-read-token endpoint, follows the redirect to
the canonical repository path, then hands the token URL to `hf-xet`. The Xet
shard cache lives in `HOARD_XET_CACHE`, or in a temporary directory that is
removed when the download finishes.

## Network rules

The token, when one is configured, is sent as an `Authorization: Bearer` header.
It is never written to error messages and it is redacted from `Debug` output.

Redirects are followed only to `huggingface.co`, hosts under `hf.co`, the Xet
CAS host, or the configured endpoint host, with at most eight hops. Plain HTTP
redirects are tolerated only on the configured endpoint host, which keeps local
mirrors and test servers usable without weakening the rule for public hosts.

Requests are retried three times on 429, 5xx, and connection or timeout
failures, with exponential backoff and jitter. Digest failures are never
retried.

## Tests

The normal test suite runs against a local `wiremock` server and never touches
the network:

```sh
cargo test -p hoard-hf --all-features
```

The live Xet gate is ignored by default because it downloads from the real Hub
and Xet CAS. It fetches `tinyllamas/stories260K.gguf` from `ggml-org/models` at
a pinned commit with a pinned digest, and fails if the transport is not Xet or
the bytes do not verify:

```sh
cargo test -p hoard-hf --test live_xet -- --ignored --nocapture
```
