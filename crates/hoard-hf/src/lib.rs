//! `hoard-hf`: Hugging Face Hub resolution and Xet or HTTP transport.
//!
//! This crate resolves a Hugging Face repository revision, lists files with
//! their source digests and Xet hashes, and downloads one file to a local path.
//! Verification against the manifest belongs to `hoard-core`.
//!
//! # Workflow
//!
//! 1. [`HubClient::resolve`] turns a branch, tag, or commit into a
//!    [`CommitSha`].
//! 2. [`HubClient::list`] fetches the recursive tree and keeps only files,
//!    carrying the LFS digest as [`RemoteFile::sha256`] and the Xet hash as
//!    [`RemoteFile::xet_hash`].
//! 3. [`HubClient::download`] moves one file to a destination, picking the Xet
//!    or HTTP transport according to [`Transport`].
//!
//! Downloads are verified against the SHA-256 digest from the tree, even when
//! the server is trusted, and are written to `<dest>.part` until verification
//! succeeds. A killed process therefore leaves a removable part file rather
//! than a plausible-looking final path.
//!
//! # Transports
//!
//! [`Transport::Auto`] prefers Xet when the file entry has a Xet hash and falls
//! back to HTTP when the Xet transfer fails. [`Transport::Xet`] and
//! [`Transport::Http`] force one transport. Files without a SHA-256 digest are
//! rejected before any request is made.
//!
//! # Configuration
//!
//! [`HubClient::new`] takes an optional endpoint and token. [`client_from_env`]
//! reads `HF_ENDPOINT`, `HF_TOKEN`, and `HUGGING_FACE_HUB_TOKEN`. Anonymous
//! access works for public repositories. The token is sent as a bearer header,
//! never written into error messages, and redacted from [`std::fmt::Debug`]
//! output. The Xet shard cache lives in `HOARD_XET_CACHE` or in a temporary
//! directory that is removed with the download.
//!
//! # Redirects and retries
//!
//! Redirects stop at the Hugging Face hosts, the Xet CAS host, or the
//! configured endpoint host, with at most eight hops. Requests are retried
//! three times on 429, 5xx, and connection or timeout failures with backoff and
//! jitter. Digest failures are never retried, because the bytes on disk are
//! known to be wrong.
//!
//! # Blocking
//!
//! The API is blocking. Call it from a normal thread, not from inside an async
//! runtime.
//!
//! ```no_run
//! use hoard_hf::{CommitSha, HubClient, RepoKind, RepoSpec, Transport};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let client = HubClient::new(None, None)?;
//! let spec = RepoSpec {
//!     kind: RepoKind::Model,
//!     repo: "ggml-org/models".into(),
//! };
//! let rev: CommitSha = "499bc8821c6b12b4e53c5bffcb21ec206f212d81".parse()?;
//! let files = client.list(&spec, &rev)?;
//! let file = files
//!     .iter()
//!     .find(|file| file.path == "tinyllamas/stories260K.gguf")
//!     .ok_or("file missing from tree")?;
//! let stats = client.download(
//!     &spec,
//!     &rev,
//!     file,
//!     std::path::Path::new("stories260K.gguf"),
//!     Transport::Auto,
//! )?;
//! println!("{} bytes over {:?}", stats.bytes, stats.transport);
//! # Ok(())
//! # }
//! ```

#![deny(missing_docs)]

mod digest;
mod error;
mod http;
mod hub;
mod redirect;
mod repo;
mod xet;

pub use error::{Error, Result};
pub use hub::{DEFAULT_ENDPOINT, DownloadStats, HubClient, RemoteFile, Transport, client_from_env};
pub use repo::{CommitSha, RepoKind, RepoSpec};

/// Crate version, as recorded by Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_not_empty() {
        assert!(!super::version().is_empty());
    }
}
