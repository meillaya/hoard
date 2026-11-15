//! The crate error type.

use std::path::PathBuf;

/// Errors returned while resolving or downloading a hub file.
///
/// Error messages never include the authentication token. URLs in messages are
/// sanitized to drop user information, and a token is only ever sent as a
/// request header.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A repository reference was not of the form `owner/name`.
    #[error("invalid repository `{0}`: expected `owner/name`")]
    InvalidRepo(String),

    /// A revision was empty.
    #[error("invalid revision `{0}`")]
    InvalidRevision(String),

    /// A commit hash was not 40 lowercase hexadecimal characters.
    #[error("invalid commit sha `{0}`: expected 40 lowercase hexadecimal characters")]
    InvalidCommit(String),

    /// The endpoint is not an absolute HTTP or HTTPS URL.
    #[error("invalid endpoint `{0}`: expected an absolute http or https URL")]
    Endpoint(String),

    /// A path in the repository tree had an empty, `.` or `..` component.
    #[error("invalid path `{0}` in repository tree")]
    InvalidPath(String),

    /// A SHA-256 digest was not 64 lowercase hexadecimal characters.
    #[error("invalid sha256 digest `{0}`: expected 64 lowercase hexadecimal characters")]
    InvalidDigest(String),

    /// An HTTP request failed before a usable response was available.
    #[error("http request failed for {context}: {source}")]
    Http {
        /// What was requested, for example `GET https://...`.
        context: String,
        /// The underlying transport error.
        #[source]
        source: reqwest::Error,
    },

    /// A response carried an unexpected status code.
    #[error("unexpected http status {status} for {context}")]
    Status {
        /// The response status code.
        status: u16,
        /// What was requested, for example `GET https://...`.
        context: String,
    },

    /// The server redirected to a location outside the allowed hosts or scheme.
    #[error("refused redirect for {context}: {message}")]
    Redirect {
        /// What was requested, for example `GET https://...`.
        context: String,
        /// Why the redirect target was rejected.
        message: String,
    },

    /// The response body was shorter than its declared `Content-Length`.
    ///
    /// Any partial data stays on disk so that the next call can resume it.
    #[error("truncated response body for {context}: expected {expected} bytes, got {actual}")]
    TruncatedBody {
        /// What was requested, for example `GET https://...`.
        context: String,
        /// The declared byte count.
        expected: u64,
        /// The byte count actually read.
        actual: u64,
    },

    /// A `206 Partial Content` response carried a `Content-Range` that does not
    /// describe the requested range.
    #[error("invalid content range `{value}` for {context}")]
    InvalidContentRange {
        /// What was requested, for example `GET https://...`.
        context: String,
        /// The offending header value, empty when the header was missing.
        value: String,
    },

    /// A file entry has no SHA-256 digest and cannot be verified.
    #[error("file `{path}` has no sha256 digest, refusing to download without verification")]
    MissingDigest {
        /// Repository path of the file.
        path: String,
    },

    /// The Xet transport needs a Xet hash.
    #[error("file `{path}` has no Xet hash, the Xet transport is not available")]
    MissingXetHash {
        /// Repository path of the file.
        path: String,
    },

    /// The downloaded file did not match the expected SHA-256.
    #[error("sha256 mismatch for `{path}`: expected {expected}, got {actual}")]
    DigestMismatch {
        /// Local destination path.
        path: PathBuf,
        /// The expected digest.
        expected: String,
        /// The digest of the downloaded bytes.
        actual: String,
    },

    /// The downloaded file did not match the expected size.
    #[error("size mismatch for `{path}`: expected {expected} bytes, got {actual}")]
    SizeMismatch {
        /// Local destination path.
        path: PathBuf,
        /// The expected size.
        expected: u64,
        /// The size of the downloaded bytes.
        actual: u64,
    },

    /// A local file operation failed.
    #[error("i/o error on `{path}`: {source}")]
    Io {
        /// The path the operation touched.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },

    /// A JSON response could not be decoded.
    #[error("could not decode JSON response for {context}: {source}")]
    Json {
        /// What was requested, for example `GET https://...`.
        context: String,
        /// The decoding error.
        #[source]
        source: serde_json::Error,
    },

    /// The Xet transfer failed.
    #[error("xet download failed for `{path}`: {message}")]
    Xet {
        /// Repository path of the file.
        path: String,
        /// The Xet session error.
        message: String,
    },

    /// `Transport::Auto` tried Xet and then HTTP, and both failed.
    #[error("xet download failed for `{path}`: {xet}; http fallback failed: {http}")]
    BothTransports {
        /// Repository path of the file.
        path: String,
        /// The Xet failure.
        xet: String,
        /// The HTTP failure.
        #[source]
        http: Box<Error>,
    },
}

impl Error {
    /// Build an [`Error::Io`] with path context.
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// A crate result with [`Error`] as the error type.
pub type Result<T> = std::result::Result<T, Error>;
