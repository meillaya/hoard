//! The shared error type for `hoard-core`.

use std::path::{Path, PathBuf};

/// A specialised [`std::result::Result`] for `hoard-core` operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned while parsing, validating, canonicalising, or hashing.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An I/O operation failed while reading a manifest or hashing a file.
    #[error("i/o error on {path}: {source}")]
    Io {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The bytes were not valid JSON for the manifest schema.
    #[error("invalid manifest JSON: {source}")]
    Json {
        /// The serde error, including the line and column of the problem.
        #[from]
        source: serde_json::Error,
    },

    /// A manifest file was not valid JSON for the manifest schema.
    #[error("invalid manifest JSON in {path}: {source}")]
    JsonFile {
        /// The manifest path.
        path: PathBuf,
        /// The serde error, including the line and column of the problem.
        #[source]
        source: serde_json::Error,
    },

    /// The manifest parsed but violates a manifest v1 rule.
    #[error("invalid manifest: {0}")]
    Validation(String),

    /// A digest string was not 64 lowercase hexadecimal characters.
    #[error("invalid digest {value:?}: {message}")]
    InvalidDigest {
        /// The offending digest string.
        value: String,
        /// What was wrong with it.
        message: String,
    },
}

impl Error {
    /// Builds an [`Error::Io`] for `path`.
    pub(crate) fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}
