//! Live gate for the Xet transport.
//!
//! This test hits the real Hugging Face Hub and the Xet CAS network, so it is
//! ignored by default. Run it explicitly:
//!
//! ```text
//! cargo test -p hoard-hf --test live_xet -- --ignored --nocapture
//! ```
//!
//! It downloads `tinyllamas/stories260K.gguf` from the model repository
//! `ggml-org/models` at a pinned revision, forcing the Xet transport and
//! checking the verified byte count and transport reported in the statistics.
//! The library verifies the SHA-256 digest before renaming the part file, so a
//! successful call already proves the bytes match the pinned digest.

use std::time::Instant;

use hoard_hf::{CommitSha, HubClient, RemoteFile, RepoKind, RepoSpec, Transport};

const REVISION: &str = "499bc8821c6b12b4e53c5bffcb21ec206f212d81";
const XET_HASH: &str = "7cfbf69e76aef0af92130af4e5c57a2a170423fb2a273b4f11a04bcc70088ba5";
const SHA256: &str = "270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d";
const SIZE: u64 = 1_185_376;
const FILE: &str = "tinyllamas/stories260K.gguf";

#[test]
#[ignore = "downloads from the live Hugging Face Hub and Xet CAS"]
fn live_xet_downloads_stories260k() {
    let client = HubClient::new(None, None).expect("anonymous Hub client");
    let spec = RepoSpec {
        kind: RepoKind::Model,
        repo: "ggml-org/models".to_string(),
    };
    let revision: CommitSha = REVISION.parse().expect("pinned revision parses");
    let file = RemoteFile {
        path: FILE.to_string(),
        size: SIZE,
        sha256: Some(SHA256.to_string()),
        xet_hash: Some(XET_HASH.to_string()),
    };

    let directory = tempfile::tempdir().expect("temporary output directory");
    let dest = directory.path().join("stories260K.gguf");
    let part = directory.path().join("stories260K.gguf.part");

    let started = Instant::now();
    let stats = client
        .download(&spec, &revision, &file, &dest, Transport::Xet)
        .expect("live Xet download");
    let elapsed = started.elapsed();

    assert_eq!(stats.transport, Transport::Xet, "gate must exercise Xet");
    assert_eq!(stats.bytes, SIZE, "byte count");
    assert_eq!(
        std::fs::metadata(&dest).expect("final file exists").len(),
        SIZE,
        "final file size"
    );
    assert!(!part.exists(), "part file must be renamed away");

    println!(
        "live xet gate passed: file={FILE} bytes={} transport={:?} wall_time={:?}",
        stats.bytes, stats.transport, elapsed
    );
}
