//! Generation and verification of the committed synthetic GGUF fixtures.
//!
//! Run with `UPDATE_FIXTURES=1` to rewrite the files:
//!
//! ```sh
//! UPDATE_FIXTURES=1 cargo test -p hoard-gguf --features testing --test generate
//! ```
//!
//! Without the variable the test verifies that the committed files are
//! byte-for-byte identical to what the fixture builder produces, so a stale
//! fixture fails CI instead of silently drifting.

#![cfg(feature = "testing")]

use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn update_requested() -> bool {
    std::env::var_os("UPDATE_FIXTURES").is_some()
}

#[test]
fn generate_synthetic_fixtures() {
    let fixtures: [(&str, Vec<u8>); 2] = [
        ("synthetic-minimal.gguf", hoard_gguf::fixture::minimal()),
        ("synthetic-rich.gguf", hoard_gguf::fixture::rich()),
    ];
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir).expect("create tests/fixtures");
    for (name, bytes) in fixtures {
        let path = dir.join(name);
        if update_requested() {
            std::fs::write(&path, &bytes).expect("write fixture");
            eprintln!("wrote {} ({} bytes)", path.display(), bytes.len());
        } else {
            let committed = std::fs::read(&path).unwrap_or_else(|error| {
                panic!(
                    "cannot read fixture {}: {error}\n\
                     run `UPDATE_FIXTURES=1 cargo test -p hoard-gguf --features testing --test generate`",
                    path.display()
                )
            });
            assert_eq!(
                committed,
                bytes,
                "{} is stale; regenerate with UPDATE_FIXTURES=1",
                path.display()
            );
        }
    }
}

#[test]
fn generated_fixtures_have_the_contracted_shape() {
    let minimal = hoard_gguf::fixture::minimal();
    assert_eq!(minimal.len(), 32);
    let gguf = hoard_gguf::Gguf::parse(&minimal).unwrap();
    assert_eq!(gguf.header().version, 3);
    assert_eq!(gguf.header().tensor_count, 0);
    assert_eq!(gguf.header().metadata_kv_count, 0);
    assert_eq!(gguf.alignment(), 32);
    assert_eq!(gguf.data_start(), 32);
    assert_eq!(gguf.tensors().len(), 0);
    assert_eq!(gguf.metadata().len(), 0);

    let rich = hoard_gguf::fixture::rich();
    let gguf = hoard_gguf::Gguf::parse(&rich).unwrap();
    assert_eq!(gguf.header().version, 3);
    assert_eq!(gguf.tensors().len(), 3);
    assert_eq!(gguf.metadata().len(), 24);
    assert_eq!(gguf.alignment(), 64);
    assert!(gguf.warnings().is_empty());
    assert_eq!(gguf.tensor_bytes().tensor_bytes, 131_496);
    // The file ends exactly at the end of the last tensor.
    let last_end = gguf
        .tensors()
        .iter()
        .map(|tensor| tensor.file_offset() + tensor.bytes().unwrap())
        .max()
        .unwrap();
    assert_eq!(last_end, rich.len() as u64);
    // The fixture is a few hundred KiB at most; keep the repository small.
    assert!(rich.len() < 256 * 1024);
}

#[test]
fn rich_fixture_json_report_and_schema_are_generated() {
    // The golden JSON and the JSON schema are verified by their own tests; this
    // one only ensures the fixture they depend on is present and parseable.
    let rich = hoard_gguf::fixture::rich();
    let gguf = hoard_gguf::Gguf::parse(&rich).unwrap();
    let report = hoard_gguf::inspect(&gguf, Some(std::path::Path::new("synthetic-rich.gguf")));
    assert_eq!(report.schema, 1);
    assert_eq!(report.tensors.count, 3);
    assert_eq!(report.metadata.count, 24);
}
