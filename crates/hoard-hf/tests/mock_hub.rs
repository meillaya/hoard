//! Mock tests for Hub resolution, listing, and both transports.
//!
//! These tests run against a local `wiremock` server. Normal `cargo test` never
//! touches the network; only the ignored `live_xet` gate does.

use hoard_hf::{CommitSha, Error, HubClient, RemoteFile, RepoKind, RepoSpec, Transport};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use url::Url;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const REVISION: &str = "499bc8821c6b12b4e53c5bffcb21ec206f212d81";
const REPO: &str = "acme/robot";
const FILE_PATH: &str = "weights/model.bin";
const CONTENT: &[u8] = b"the quick brown fox jumps over the lazy dog\n";

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn spec() -> RepoSpec {
    RepoSpec {
        kind: RepoKind::Model,
        repo: REPO.to_string(),
    }
}

fn revision() -> CommitSha {
    REVISION.parse().expect("pinned revision parses")
}

fn model_url_path(file: &str) -> String {
    format!("/{REPO}/resolve/{REVISION}/{file}")
}

fn remote_file(sha256: String) -> RemoteFile {
    RemoteFile {
        path: FILE_PATH.to_string(),
        size: CONTENT.len() as u64,
        sha256: Some(sha256),
        xet_hash: None,
    }
}

fn client_for(uri: &str, token: Option<String>) -> HubClient {
    HubClient::new(Some(Url::parse(uri).expect("mock server url")), token).expect("hub client")
}

/// Run a blocking operation on the blocking pool.
///
/// The blocking client owns a background runtime, so it must be built and
/// dropped on a blocking thread rather than on the async test thread.
async fn on_blocking<T, F>(work: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .expect("blocking task")
}

async fn resolve(
    server: &MockServer,
    spec: RepoSpec,
    revision: &'static str,
) -> Result<CommitSha, Error> {
    let uri = server.uri();
    on_blocking(move || client_for(&uri, None).resolve(&spec, revision)).await
}

async fn resolve_with_token(
    server: &MockServer,
    token: &'static str,
    spec: RepoSpec,
) -> Result<CommitSha, Error> {
    let uri = server.uri();
    on_blocking(move || client_for(&uri, Some(token.to_string())).resolve(&spec, "main")).await
}

async fn list(server: &MockServer) -> Result<Vec<RemoteFile>, Error> {
    let uri = server.uri();
    let spec = spec();
    let revision = revision();
    on_blocking(move || client_for(&uri, None).list(&spec, &revision)).await
}

async fn download(
    server: &MockServer,
    file: RemoteFile,
    dest: std::path::PathBuf,
    mode: Transport,
) -> (Result<hoard_hf::DownloadStats, Error>, std::path::PathBuf) {
    let uri = server.uri();
    let spec = spec();
    let revision = revision();
    on_blocking(move || {
        let client = client_for(&uri, None);
        let stats = client.download(&spec, &revision, &file, &dest, mode);
        (stats, dest)
    })
    .await
}

fn part_of(dest: &std::path::Path) -> std::path::PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(".part");
    std::path::PathBuf::from(name)
}

fn destination() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().expect("temp dir");
    let dest = directory.path().join("model.bin");
    (directory, dest)
}

#[tokio::test]
async fn resolve_returns_commit_sha() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/models/{REPO}/revision/main")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "sha": REVISION })))
        .mount(&server)
        .await;

    let sha = resolve(&server, spec(), "main").await.expect("resolve");
    assert_eq!(sha.as_str(), REVISION);
    assert_eq!(sha.to_string(), REVISION);
}

#[tokio::test]
async fn resolve_rejects_a_response_that_is_not_a_commit_sha() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/models/{REPO}/revision/main")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "sha": "main" })))
        .mount(&server)
        .await;

    let error = resolve(&server, spec(), "main")
        .await
        .expect_err("a branch name is not a resolved commit");
    assert!(matches!(error, Error::InvalidCommit(_)), "{error:?}");
}

#[tokio::test]
async fn list_filters_directories_and_maps_digests() {
    let server = MockServer::start().await;
    let digest = sha256_hex(CONTENT);
    let tree_path = format!("/api/models/{REPO}/tree/{REVISION}");
    Mock::given(method("GET"))
        .and(path(tree_path))
        .and(query_param("recursive", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "type": "directory", "path": "weights" },
            {
                "type": "file",
                "path": FILE_PATH,
                "size": CONTENT.len(),
                "lfs": { "oid": digest, "size": CONTENT.len() },
                "xetHash": "7cfbf69e76aef0af92130af4e5c57a2a170423fb2a273b4f11a04bcc70088ba5"
            },
            { "type": "file", "path": "README.md", "size": 5 }
        ])))
        .mount(&server)
        .await;

    let files = list(&server).await.expect("list");
    assert_eq!(files.len(), 2, "directory entries are filtered out");
    assert_eq!(files[0].path, "README.md", "entries sort by path");
    assert_eq!(files[0].sha256, None);
    assert_eq!(files[1].path, FILE_PATH);
    assert_eq!(files[1].size, CONTENT.len() as u64);
    assert_eq!(files[1].sha256.as_deref(), Some(digest.as_str()));
    assert!(files[1].xet_hash.is_some());
}

#[tokio::test]
async fn not_found_status_is_reported() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/models/{REPO}/revision/main")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let error = resolve(&server, spec(), "main")
        .await
        .expect_err("a missing revision is an error");
    assert!(
        matches!(error, Error::Status { status: 404, .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn retries_429_then_succeeds() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_mock = calls.clone();
    Mock::given(method("GET"))
        .and(path(format!("/api/models/{REPO}/revision/main")))
        .respond_with(move |_request: &Request| {
            if calls_for_mock.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(429)
            } else {
                ResponseTemplate::new(200).set_body_json(json!({ "sha": REVISION }))
            }
        })
        .mount(&server)
        .await;

    let sha = resolve(&server, spec(), "main")
        .await
        .expect("the retry succeeds");
    assert_eq!(sha.as_str(), REVISION);
    assert_eq!(calls.load(Ordering::SeqCst), 2, "exactly one retry");
}

#[tokio::test]
async fn retries_5xx_then_succeeds() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_mock = calls.clone();
    Mock::given(method("GET"))
        .and(path(format!("/api/models/{REPO}/revision/main")))
        .respond_with(move |_request: &Request| {
            if calls_for_mock.fetch_add(1, Ordering::SeqCst) < 2 {
                ResponseTemplate::new(503)
            } else {
                ResponseTemplate::new(200).set_body_json(json!({ "sha": REVISION }))
            }
        })
        .mount(&server)
        .await;

    let sha = resolve(&server, spec(), "main")
        .await
        .expect("two retries suffice");
    assert_eq!(sha.as_str(), REVISION);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn redirect_to_allowed_host_is_followed() {
    let server = MockServer::start().await;
    let canonical = "/canonical/revision/main".to_string();
    Mock::given(method("GET"))
        .and(path(format!("/api/models/{REPO}/revision/main")))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("Location", format!("{}{canonical}", server.uri())),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(canonical))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "sha": REVISION })))
        .mount(&server)
        .await;

    let sha = resolve(&server, spec(), "main")
        .await
        .expect("the endpoint host is an allowed redirect target");
    assert_eq!(sha.as_str(), REVISION);
}

#[tokio::test]
async fn redirect_to_foreign_host_is_refused() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/models/{REPO}/revision/main")))
        .respond_with(ResponseTemplate::new(307).insert_header(
            "Location",
            "https://evil.invalid/api/models/acme/robot/revision/main",
        ))
        .mount(&server)
        .await;

    let error = resolve(&server, spec(), "main")
        .await
        .expect_err("a foreign redirect target must be refused");
    assert!(matches!(error, Error::Redirect { .. }), "{error:?}");
    let message = error.to_string();
    assert!(message.contains("refused redirect"), "{message}");
    assert!(message.contains("evil.invalid"), "{message}");
}

#[tokio::test]
async fn truncated_body_is_an_error_and_leaves_no_final_file() {
    let server = MockServer::start().await;
    // Claim the full length, deliver only the first ten bytes.
    let template = ResponseTemplate::new(200)
        .insert_header("Content-Length", CONTENT.len().to_string())
        .set_body_bytes(&CONTENT[..10]);
    Mock::given(method("GET"))
        .and(path(model_url_path(FILE_PATH)))
        .respond_with(template)
        .mount(&server)
        .await;

    let (_directory, dest) = destination();
    let file = remote_file(sha256_hex(CONTENT));
    let (result, dest) = download(&server, file, dest.clone(), Transport::Http).await;

    assert!(result.is_err(), "a short body must not verify");
    assert!(!dest.exists(), "no final file after a truncated body");
}

#[tokio::test]
async fn wrong_sha256_is_rejected_and_the_part_file_is_removed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(model_url_path(FILE_PATH)))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(CONTENT))
        .mount(&server)
        .await;

    let (_directory, dest) = destination();
    let file = remote_file(sha256_hex(b"a different content"));
    let (result, dest) = download(&server, file, dest.clone(), Transport::Http).await;

    let error = result.expect_err("the digest does not match");
    assert!(matches!(error, Error::DigestMismatch { .. }), "{error:?}");
    assert!(!dest.exists(), "no final file after a digest mismatch");
    assert!(!part_of(&dest).exists(), "a poisoned part file is removed");
}

#[tokio::test]
async fn http_range_206_resumes_a_partial_file() {
    let server = MockServer::start().await;
    let resume_at = 20usize;
    let (prefix, rest) = CONTENT.split_at(resume_at);
    Mock::given(method("GET"))
        .and(path(model_url_path(FILE_PATH)))
        .and(header("range", format!("bytes={resume_at}-")))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header(
                    "Content-Range",
                    format!(
                        "bytes {resume_at}-{last}/{total}",
                        last = CONTENT.len() - 1,
                        total = CONTENT.len()
                    ),
                )
                .set_body_bytes(rest),
        )
        .mount(&server)
        .await;

    let (_directory, dest) = destination();
    std::fs::write(part_of(&dest), prefix).expect("seed partial file");
    let file = remote_file(sha256_hex(CONTENT));
    let (result, dest) = download(&server, file, dest.clone(), Transport::Http).await;

    let stats = result.expect("the 206 completes the file");
    assert_eq!(stats.transport, Transport::Http);
    assert_eq!(stats.bytes, CONTENT.len() as u64);
    assert_eq!(std::fs::read(&dest).expect("final file"), CONTENT);
    assert!(!part_of(&dest).exists());
}

#[tokio::test]
async fn http_200_restarts_when_the_server_ignores_the_range() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(model_url_path(FILE_PATH)))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(CONTENT))
        .mount(&server)
        .await;

    let (_directory, dest) = destination();
    std::fs::write(part_of(&dest), &CONTENT[..10]).expect("seed stale partial file");
    let file = remote_file(sha256_hex(CONTENT));
    let (result, dest) = download(&server, file, dest.clone(), Transport::Http).await;

    result.expect("the full body replaces the stale part");
    assert_eq!(std::fs::read(&dest).expect("final file"), CONTENT);
    assert!(!part_of(&dest).exists());
}

#[tokio::test]
async fn auto_without_a_xet_hash_uses_http() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(model_url_path(FILE_PATH)))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(CONTENT))
        .mount(&server)
        .await;

    let (_directory, dest) = destination();
    let file = remote_file(sha256_hex(CONTENT));
    let (result, dest) = download(&server, file, dest.clone(), Transport::Auto).await;

    let stats = result.expect("auto falls back to http without a xet hash");
    assert_eq!(stats.transport, Transport::Http);
    assert_eq!(stats.bytes, CONTENT.len() as u64);
    assert!(dest.exists());
}

#[test]
fn forced_xet_without_a_xet_hash_is_rejected_before_any_request() {
    let client = HubClient::new(None, None).expect("client");
    let (_directory, dest) = destination();
    let file = remote_file(sha256_hex(CONTENT));
    let error = client
        .download(&spec(), &revision(), &file, &dest, Transport::Xet)
        .expect_err("xet needs a xet hash");
    assert!(matches!(error, Error::MissingXetHash { .. }), "{error:?}");
    assert!(!dest.exists());
}

#[test]
fn file_without_a_digest_is_rejected_before_any_request() {
    let client = HubClient::new(None, None).expect("client");
    let (_directory, dest) = destination();
    let file = RemoteFile {
        path: FILE_PATH.to_string(),
        size: CONTENT.len() as u64,
        sha256: None,
        xet_hash: Some("7cfbf69e76aef0af92130af4e5c57a2a170423fb2a273b4f11a04bcc70088ba5".into()),
    };
    let error = client
        .download(&spec(), &revision(), &file, &dest, Transport::Auto)
        .expect_err("an unverifiable file must not be downloaded");
    assert!(matches!(error, Error::MissingDigest { .. }), "{error:?}");
    assert!(!dest.exists());
}

#[test]
fn file_with_a_malformed_digest_is_rejected_before_any_request() {
    let client = HubClient::new(None, None).expect("client");
    let (_directory, dest) = destination();
    let file = remote_file("not-a-digest".to_string());
    let error = client
        .download(&spec(), &revision(), &file, &dest, Transport::Http)
        .expect_err("a malformed digest is rejected");
    assert!(matches!(error, Error::InvalidDigest(_)), "{error:?}");
}

#[tokio::test]
async fn the_token_is_sent_as_a_bearer_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/models/{REPO}/revision/main")))
        .and(header("authorization", "Bearer hf_secret_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "sha": REVISION })))
        .mount(&server)
        .await;

    let sha = resolve_with_token(&server, "hf_secret_token", spec())
        .await
        .expect("the bearer header is accepted");
    assert_eq!(sha.as_str(), REVISION);
}

#[tokio::test]
async fn connection_errors_are_retried_before_failing() {
    // Bind a port and free it again, so connecting to it is refused.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback port");
    let address = listener.local_addr().expect("local address");
    drop(listener);
    let uri = format!("http://{address}");

    let (error, elapsed) = on_blocking(move || {
        let client = client_for(&uri, None);
        let started = std::time::Instant::now();
        let error = client
            .resolve(&spec(), "main")
            .expect_err("nothing listens on the port");
        (error, started.elapsed())
    })
    .await;

    assert!(matches!(error, Error::Http { .. }), "{error:?}");
    // Three retries at 100 ms, 200 ms, and 400 ms of backoff, plus jitter.
    assert!(
        elapsed >= std::time::Duration::from_millis(600),
        "retries did not back off: {elapsed:?}"
    );
}
