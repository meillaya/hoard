//! `hoard-core` — manifest v1, canonical JSON, validation, and hashing for the
//! `hoard` artifact layer.
//!
//! A manifest records where an artifact set came from (a Hugging Face
//! repository at a pinned commit) and the files that make up the set. Every
//! file carries its SHA-256 as published by the source and its BLAKE3 as
//! computed by hoard, the digest used for content-addressed storage. Manifests
//! have exactly one canonical JSON encoding, and the manifest hash is the
//! BLAKE3 of that encoding.
//!
//! # Example
//!
//! ```
//! # fn main() -> Result<(), hoard_core::Error> {
//! use hoard_core::Manifest;
//!
//! let json = br#"{
//!   "files": [{
//!     "blake3": "0000000000000000000000000000000000000000000000000000000000000001",
//!     "path": "tinyllamas/stories260K.gguf",
//!     "role": "weights",
//!     "sha256": "0000000000000000000000000000000000000000000000000000000000000002",
//!     "size": 1185376
//!   }],
//!   "schema": 1,
//!   "source": {
//!     "kind": "huggingface",
//!     "repo": "ggml-org/models",
//!     "repoType": "model",
//!     "revision": "499bc8821c6b12b4e53c5bffcb21ec206f212d81"
//!   }
//! }"#;
//!
//! let manifest = Manifest::from_json_bytes(json)?;
//! manifest.validate()?;
//!
//! let canonical = manifest.canonical_bytes();
//! assert!(canonical.ends_with(b"\n"));
//! println!("manifest hash: {}", manifest.hash());
//! # Ok(())
//! # }
//! ```
//!
//! # Canonical form
//!
//! Object keys are sorted lexicographically at every level, indentation is two
//! spaces, line endings are LF, and the document has exactly one trailing
//! newline. Files are sorted by path and sibling lists are sorted. Use
//! [`Manifest::canonical_bytes`] for the bytes; do not hand-format manifests.
//!
//! # Modules
//!
//! - [`manifest`] — the manifest v1 types.
//! - [`validate`] — the manifest v1 rules.
//! - [`canonical`] — the canonical JSON encoding.
//! - [`hash`] — streaming SHA-256/BLAKE3 helpers and digest types.
//! - [`schema`] — the JSON Schema for manifest v1.
//! - [`error`] — the shared error type.

#![deny(missing_docs)]

pub mod canonical;
pub mod error;
pub mod hash;
pub mod manifest;
pub mod schema;
pub mod validate;

pub use error::{Error, Result};
pub use hash::{Blake3Digest, Hash32};
pub use manifest::{FileEntry, Manifest, RepoType, Role, SCHEMA_VERSION, Source, SourceKind};
pub use schema::manifest_schema;

/// Crate version, as recorded by Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
