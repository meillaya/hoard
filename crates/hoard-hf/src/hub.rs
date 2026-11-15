//! The Hub client: resolution, listing, and transport dispatch.

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::Url;
use reqwest::blocking::{Client, Request, Response};
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::digest;
use crate::error::{Error, Result};
use crate::http;
use crate::redirect;
use crate::repo::{CommitSha, RepoSpec};
use crate::xet;

/// The public Hugging Face Hub endpoint used when none is configured.
pub const DEFAULT_ENDPOINT: &str = "https://huggingface.co";

/// One attempt plus three retries.
const MAX_ATTEMPTS: usize = 4;

/// Base delay for a retry; doubled per attempt with jitter added.
const RETRY_BASE_DELAY: Duration = Duration::from_millis(100);

const USER_AGENT_VALUE: &str = concat!("hoard-hf/", env!("CARGO_PKG_VERSION"));

/// A file listed in a repository tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    /// Path inside the repository, using `/` separators.
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// Git LFS object id: the file's lowercase hexadecimal SHA-256.
    ///
    /// Files that do not use LFS have no digest here.
    pub sha256: Option<String>,
    /// Xet content hash, when the Hub stored the file with Xet.
    pub xet_hash: Option<String>,
}

/// The transport to use for a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Transport {
    /// Use Xet when the file has a Xet hash, otherwise HTTP.
    ///
    /// If the Xet transfer fails the client falls back to HTTP and the returned
    /// statistics report [`Transport::Http`].
    Auto,
    /// Force the Xet transport.
    Xet,
    /// Force the plain HTTP transport.
    Http,
}

/// Statistics for one completed download.
#[derive(Debug, Clone)]
pub struct DownloadStats {
    /// Number of bytes written, equal to the verified file size.
    pub bytes: u64,
    /// The transport actually used.
    pub transport: Transport,
    /// Wall-clock time spent resolving the transport and downloading.
    pub elapsed: Duration,
}

/// A blocking client for the Hugging Face Hub.
///
/// The client holds the endpoint, the optional token, and one pooled HTTP
/// client. It is cheap to clone. All methods block, so call them from a normal
/// thread rather than from inside an async runtime.
#[derive(Clone)]
pub struct HubClient {
    endpoint: Url,
    endpoint_host: String,
    token: Option<String>,
    client: Client,
}

impl std::fmt::Debug for HubClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubClient")
            .field("endpoint", &redirect::display_url(&self.endpoint))
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish_non_exhaustive()
    }
}

impl HubClient {
    /// Create a client for `endpoint` (default: <https://huggingface.co>) with
    /// an optional access token.
    pub fn new(endpoint: Option<Url>, token: Option<String>) -> Result<Self> {
        let endpoint = match endpoint {
            Some(endpoint) => validate_endpoint(endpoint)?,
            None => Url::parse(DEFAULT_ENDPOINT).expect("the default endpoint is a valid URL"),
        };
        let endpoint_host = endpoint
            .host_str()
            .ok_or_else(|| Error::Endpoint(redirect::display_url(&endpoint)))?
            .to_string();
        let token = token
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty());

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(token) = &token {
            let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
                Error::Endpoint("the access token is not a valid header value".into())
            })?;
            headers.insert(AUTHORIZATION, value);
        }

        let client = Client::builder()
            .user_agent(USER_AGENT_VALUE)
            .default_headers(headers)
            .redirect(redirect::policy(endpoint_host.clone()))
            .connect_timeout(Duration::from_secs(30))
            .build()
            .map_err(|source| Error::Http {
                context: "building the http client".to_string(),
                source,
            })?;

        Ok(Self {
            endpoint,
            endpoint_host,
            token,
            client,
        })
    }

    /// The configured endpoint.
    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    /// Resolve `rev` (a branch, tag, or commit) to a commit hash.
    pub fn resolve(&self, spec: &RepoSpec, rev: &str) -> Result<CommitSha> {
        if rev.is_empty() {
            return Err(Error::InvalidRevision(rev.to_string()));
        }
        let url = self.revision_url(spec, rev)?;
        let context = format!("GET {}", redirect::display_url(&url));
        let body: RevisionResponse = self.get_json(&url, &context)?;
        CommitSha::from_str(&body.sha)
    }

    /// List the files in a repository at a resolved commit.
    ///
    /// Directories are filtered out; entries are sorted by path.
    pub fn list(&self, spec: &RepoSpec, rev: &CommitSha) -> Result<Vec<RemoteFile>> {
        let url = self.tree_url(spec, rev)?;
        let context = format!("GET {}", redirect::display_url(&url));
        let entries: Vec<TreeEntry> = self.get_json(&url, &context)?;

        let mut files: Vec<RemoteFile> = entries
            .into_iter()
            .filter(|entry| entry.kind == "file")
            .map(|entry| RemoteFile {
                path: entry.path,
                size: entry.size.unwrap_or(0),
                sha256: entry.lfs.map(|lfs| lfs.oid),
                xet_hash: entry.xet_hash,
            })
            .collect();
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    /// Download one file to `dest`.
    ///
    /// The file is written to `<dest>.part`, verified against the entry's
    /// SHA-256 and size, and only then renamed to `dest`. Failures leave the
    /// final path absent; a digest mismatch removes the partial file, while a
    /// truncated body keeps it so the next call can resume.
    pub fn download(
        &self,
        spec: &RepoSpec,
        rev: &CommitSha,
        file: &RemoteFile,
        dest: &Path,
        mode: Transport,
    ) -> Result<DownloadStats> {
        let sha256 = file.sha256.as_deref().ok_or_else(|| Error::MissingDigest {
            path: file.path.clone(),
        })?;
        digest::decode_sha256(sha256)?;
        let started = Instant::now();

        let (bytes, transport) = match mode {
            Transport::Http => (
                http::download_file(self, spec, rev, file, dest, sha256)?,
                Transport::Http,
            ),
            Transport::Xet => (
                xet::download_file(self, spec, rev, file, dest, sha256)?,
                Transport::Xet,
            ),
            Transport::Auto => {
                if file.xet_hash.is_some() {
                    match xet::download_file(self, spec, rev, file, dest, sha256) {
                        Ok(bytes) => (bytes, Transport::Xet),
                        Err(xet_error) if fallback_allowed(&xet_error) => {
                            match http::download_file(self, spec, rev, file, dest, sha256) {
                                Ok(bytes) => (bytes, Transport::Http),
                                Err(http_error) => {
                                    return Err(Error::BothTransports {
                                        path: file.path.clone(),
                                        xet: xet_error.to_string(),
                                        http: Box::new(http_error),
                                    });
                                }
                            }
                        }
                        // Verification failures are not retried on another
                        // transport: the bytes were wrong, not the connection.
                        Err(xet_error) => return Err(xet_error),
                    }
                } else {
                    (
                        http::download_file(self, spec, rev, file, dest, sha256)?,
                        Transport::Http,
                    )
                }
            }
        };

        Ok(DownloadStats {
            bytes,
            transport,
            elapsed: started.elapsed(),
        })
    }

    /// The bearer token, when one was configured.
    pub(crate) fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Build a `GET` request for `url` through the shared client.
    pub(crate) fn request(&self, url: &Url) -> Result<Request> {
        self.client
            .get(url.clone())
            .build()
            .map_err(|source| Error::Http {
                context: format!("building GET {}", redirect::display_url(url)),
                source,
            })
    }

    /// Execute a request, retrying on 429, 5xx, and connection or timeout
    /// failures. `build` is called before every attempt.
    pub(crate) fn send_with_retry<F>(&self, context: &str, build: F) -> Result<Response>
    where
        F: Fn() -> Result<Request>,
    {
        let mut attempt = 1;
        loop {
            match self.client.execute(build()?) {
                Ok(response) => {
                    let status = response.status();
                    let retryable = status.as_u16() == 429 || status.is_server_error();
                    if retryable && attempt < MAX_ATTEMPTS {
                        std::thread::sleep(backoff(attempt));
                        attempt += 1;
                        continue;
                    }
                    return Ok(response);
                }
                Err(error) => {
                    let retryable = error.is_timeout() || error.is_connect();
                    if retryable && attempt < MAX_ATTEMPTS {
                        std::thread::sleep(backoff(attempt));
                        attempt += 1;
                        continue;
                    }
                    return Err(map_reqwest_error(error, context));
                }
            }
        }
    }

    /// Fetch a URL, follow redirects, and return the final URL without
    /// consuming the response body.
    pub(crate) fn canonical_redirect(&self, url: &Url, context: &str) -> Result<Url> {
        let response = self.send_with_retry(context, || self.request(url))?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Status {
                status: status.as_u16(),
                context: context.to_string(),
            });
        }
        let final_url = response.url().clone();
        if !redirect::target_allowed(&final_url, &self.endpoint_host) {
            return Err(Error::Redirect {
                context: context.to_string(),
                message: format!(
                    "final location {} is not allowed",
                    redirect::display_url(&final_url)
                ),
            });
        }
        Ok(final_url)
    }

    /// The Xet read-token URL for a file, after redirects to the canonical
    /// repository have been followed.
    pub(crate) fn canonical_xet_token_url(&self, spec: &RepoSpec, rev: &CommitSha) -> Result<Url> {
        let url = self.xet_token_url(spec, rev)?;
        let context = format!("GET {}", redirect::display_url(&url));
        self.canonical_redirect(&url, &context)
    }

    /// Resolve a revision to a full commit hash.
    fn revision_url(&self, spec: &RepoSpec, rev: &str) -> Result<Url> {
        let (owner, name) = spec.split()?;
        let plural = format!("{}s", spec.kind.api_segment());
        join_path(
            &self.endpoint,
            &["api", &plural, owner, name, "revision", rev],
        )
    }

    /// The recursive tree URL for a resolved commit.
    fn tree_url(&self, spec: &RepoSpec, rev: &CommitSha) -> Result<Url> {
        let (owner, name) = spec.split()?;
        let plural = format!("{}s", spec.kind.api_segment());
        let mut url = join_path(
            &self.endpoint,
            &["api", &plural, owner, name, "tree", rev.as_str()],
        )?;
        url.query_pairs_mut().append_pair("recursive", "true");
        Ok(url)
    }

    /// The Xet read-token URL for a resolved commit.
    fn xet_token_url(&self, spec: &RepoSpec, rev: &CommitSha) -> Result<Url> {
        let (owner, name) = spec.split()?;
        let plural = format!("{}s", spec.kind.api_segment());
        join_path(
            &self.endpoint,
            &["api", &plural, owner, name, "xet-read-token", rev.as_str()],
        )
    }

    /// The plain HTTP file URL for a resolved commit.
    pub(crate) fn http_file_url(
        &self,
        spec: &RepoSpec,
        rev: &CommitSha,
        path: &str,
    ) -> Result<Url> {
        let (owner, name) = spec.split()?;
        let mut segments: Vec<&str> = Vec::new();
        let prefix = spec.kind.file_prefix();
        if !prefix.is_empty() {
            segments.push(prefix);
        }
        segments.push(owner);
        segments.push(name);
        segments.push("resolve");
        segments.push(rev.as_str());
        for component in path.split('/') {
            if component.is_empty() || component == "." || component == ".." {
                return Err(Error::InvalidPath(path.to_string()));
            }
            segments.push(component);
        }
        join_path(&self.endpoint, &segments)
    }

    /// Send a GET and decode a JSON body.
    fn get_json<T: DeserializeOwned>(&self, url: &Url, context: &str) -> Result<T> {
        let response = self.send_with_retry(context, || self.request(url))?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Status {
                status: status.as_u16(),
                context: context.to_string(),
            });
        }
        let bytes = response.bytes().map_err(|source| Error::Http {
            context: format!("reading the response body for {context}"),
            source,
        })?;
        serde_json::from_slice(&bytes).map_err(|source| Error::Json {
            context: context.to_string(),
            source,
        })
    }
}

/// Read the endpoint and token from the environment.
///
/// Recognized variables are `HF_ENDPOINT` for the endpoint and `HF_TOKEN` or
/// `HUGGING_FACE_HUB_TOKEN` for the token. Anonymous access is used when no
/// token is set.
pub fn client_from_env() -> Result<HubClient> {
    let endpoint = match std::env::var("HF_ENDPOINT") {
        Ok(value) if !value.trim().is_empty() => {
            let value = value.trim();
            Some(Url::parse(value).map_err(|_| Error::Endpoint(value.to_string()))?)
        }
        _ => None,
    };
    let token = ["HF_TOKEN", "HUGGING_FACE_HUB_TOKEN"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok())
        .filter(|value| !value.trim().is_empty());
    HubClient::new(endpoint, token)
}

/// The part path for a destination: the full path plus a `.part` suffix.
pub(crate) fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

/// Create the parent directory of `dest`, when it has one.
pub(crate) fn ensure_parent(dest: &Path) -> Result<()> {
    match dest.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))
        }
        _ => Ok(()),
    }
}

/// Verify a completed part file and move it to its destination.
///
/// A verification failure removes the part file, because the bytes on disk are
/// known to be wrong.
pub(crate) fn finish_download(
    part: &Path,
    dest: &Path,
    expected_size: u64,
    expected_sha: &str,
) -> Result<u64> {
    match digest::verify_file(part, expected_size, expected_sha) {
        Ok(size) => {
            fs::rename(part, dest).map_err(|error| Error::io(dest, error))?;
            Ok(size)
        }
        Err(error) => {
            let _ = fs::remove_file(part);
            Err(error)
        }
    }
}

/// Sleep before retry attempt `attempt` (1-based): exponential backoff with up
/// to 100 ms of jitter.
fn backoff(attempt: usize) -> Duration {
    let factor = 1u32 << attempt.saturating_sub(1).min(8);
    let base = RETRY_BASE_DELAY.saturating_mul(factor);
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos() % 100)
        .unwrap_or(0);
    base + Duration::from_millis(u64::from(jitter))
}

/// Whether an Xet failure is a transport problem that may be retried over HTTP.
///
/// A digest or size mismatch means the downloaded bytes were wrong, so another
/// transport must not retry the same file.
fn fallback_allowed(error: &Error) -> bool {
    !matches!(
        error,
        Error::DigestMismatch { .. } | Error::SizeMismatch { .. } | Error::InvalidDigest(_)
    )
}

fn map_reqwest_error(error: reqwest::Error, context: &str) -> Error {
    if error.is_redirect() {
        Error::Redirect {
            context: context.to_string(),
            message: describe_error_chain(&error),
        }
    } else {
        Error::Http {
            context: context.to_string(),
            source: error,
        }
    }
}

/// Render an error together with its source chain, so a redirect policy reason
/// is not hidden behind reqwest's top-level message.
fn describe_error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(cause) = source {
        parts.push(cause.to_string());
        source = cause.source();
    }
    parts.join(": ")
}

fn validate_endpoint(endpoint: Url) -> Result<Url> {
    match endpoint.scheme() {
        "http" | "https" if endpoint.host_str().is_some() => Ok(endpoint),
        _ => Err(Error::Endpoint(redirect::display_url(&endpoint))),
    }
}

/// Append path segments to a base URL.
pub(crate) fn join_path(base: &Url, segments: &[&str]) -> Result<Url> {
    let mut url = base.clone();
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| Error::Endpoint(redirect::display_url(base)))?;
        path.pop_if_empty();
        for segment in segments {
            path.push(segment);
        }
    }
    Ok(url)
}

#[derive(Debug, Deserialize)]
struct RevisionResponse {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct TreeEntry {
    #[serde(rename = "type")]
    kind: String,
    path: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    lfs: Option<LfsEntry>,
    #[serde(default, rename = "xetHash")]
    xet_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LfsEntry {
    oid: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::RepoKind;

    fn client() -> HubClient {
        HubClient::new(None, None).unwrap()
    }

    fn spec() -> RepoSpec {
        RepoSpec {
            kind: RepoKind::Model,
            repo: "ggml-org/models".to_string(),
        }
    }

    fn sha() -> CommitSha {
        "499bc8821c6b12b4e53c5bffcb21ec206f212d81".parse().unwrap()
    }

    #[test]
    fn default_endpoint_is_hugging_face() {
        assert_eq!(client().endpoint().as_str(), "https://huggingface.co/");
    }

    #[test]
    fn debug_redacts_the_token() {
        let client = HubClient::new(None, Some("hf_very_secret_value".to_string())).unwrap();
        let rendered = format!("{client:?}");
        assert!(!rendered.contains("hf_very_secret_value"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn urls_match_the_hub_routes() {
        let client = client();
        assert_eq!(
            client.revision_url(&spec(), "main").unwrap().as_str(),
            "https://huggingface.co/api/models/ggml-org/models/revision/main"
        );
        assert_eq!(
            client.tree_url(&spec(), &sha()).unwrap().as_str(),
            "https://huggingface.co/api/models/ggml-org/models/tree/499bc8821c6b12b4e53c5bffcb21ec206f212d81?recursive=true"
        );
        assert_eq!(
            client.xet_token_url(&spec(), &sha()).unwrap().as_str(),
            "https://huggingface.co/api/models/ggml-org/models/xet-read-token/499bc8821c6b12b4e53c5bffcb21ec206f212d81"
        );
    }

    #[test]
    fn model_resolve_url_has_no_prefix() {
        let client = client();
        let file = RemoteFile {
            path: "tinyllamas/stories260K.gguf".to_string(),
            size: 1,
            sha256: None,
            xet_hash: None,
        };
        assert_eq!(
            client
                .http_file_url(&spec(), &sha(), &file.path)
                .unwrap()
                .as_str(),
            "https://huggingface.co/ggml-org/models/resolve/499bc8821c6b12b4e53c5bffcb21ec206f212d81/tinyllamas/stories260K.gguf"
        );
    }

    #[test]
    fn dataset_and_space_resolve_urls_are_prefixed() {
        let client = client();
        let sha = sha();
        for (kind, prefix) in [(RepoKind::Dataset, "datasets"), (RepoKind::Space, "spaces")] {
            let spec = RepoSpec {
                kind,
                repo: "acme/data".to_string(),
            };
            assert_eq!(
                client
                    .http_file_url(&spec, &sha, "dir/file.bin")
                    .unwrap()
                    .as_str(),
                format!("https://huggingface.co/{prefix}/acme/data/resolve/{sha}/dir/file.bin")
            );
        }
    }

    #[test]
    fn file_paths_are_validated() {
        let client = client();
        for bad in ["", "dir//file", "dir/../file", "./file"] {
            assert!(
                client.http_file_url(&spec(), &sha(), bad).is_err(),
                "accepted {bad:?}"
            );
        }
    }

    #[test]
    fn part_path_appends_suffix() {
        let part = part_path(Path::new("/tmp/model.gguf"));
        assert_eq!(part, Path::new("/tmp/model.gguf.part"));
    }

    #[test]
    fn verification_failures_are_not_retried_on_http() {
        assert!(!fallback_allowed(&Error::DigestMismatch {
            path: PathBuf::from("/tmp/model.part"),
            expected: "a".repeat(64),
            actual: "b".repeat(64),
        }));
        assert!(!fallback_allowed(&Error::SizeMismatch {
            path: PathBuf::from("/tmp/model.part"),
            expected: 10,
            actual: 9,
        }));
        assert!(fallback_allowed(&Error::Status {
            status: 404,
            context: "xet token".into(),
        }));
        assert!(fallback_allowed(&Error::Xet {
            path: "weights/model.bin".into(),
            message: "temporary CAS failure".into(),
        }));
    }

    #[test]
    fn repos_need_owner_and_name() {
        let client = client();
        let spec = RepoSpec {
            kind: RepoKind::Model,
            repo: "missing-slash".to_string(),
        };
        assert!(matches!(
            client.revision_url(&spec, "main"),
            Err(Error::InvalidRepo(_))
        ));
    }

    #[test]
    fn new_normalizes_tokens_and_rejects_bad_endpoints() {
        let client = HubClient::new(None, Some("  ".to_string())).unwrap();
        assert!(client.token().is_none());

        assert!(matches!(
            HubClient::new(Some(Url::parse("ftp://example.com").unwrap()), None),
            Err(Error::Endpoint(_))
        ));
        assert!(matches!(
            HubClient::new(
                Some(Url::parse("mailto:someone@example.com").unwrap()),
                None
            ),
            Err(Error::Endpoint(_))
        ));
    }
}
