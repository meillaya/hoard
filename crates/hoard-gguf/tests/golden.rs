//! Golden test: the `inspect` JSON for the synthetic rich fixture is frozen.
//!
//! The golden document is committed at
//! `tests/golden/inspect-synthetic-rich-v1.json`. Regenerate it with:
//!
//! ```sh
//! UPDATE_GOLDEN=1 cargo test -p hoard-gguf --test golden
//! ```
//!
//! Any change to the report shape, field order, number formatting, or warning
//! messages fails this test until the golden file is deliberately updated.

mod common;

use std::fs;
use std::path::Path;

use hoard_gguf::{Gguf, inspect};

fn golden_path() -> std::path::PathBuf {
    common::golden_dir().join("inspect-synthetic-rich-v1.json")
}

fn rendered_report() -> String {
    let bytes = common::rich_gguf();
    let gguf = Gguf::parse(&bytes).expect("synthetic-rich.gguf parses");
    let report = inspect(&gguf, Some(Path::new("synthetic-rich.gguf")));
    let mut rendered = serde_json::to_string_pretty(&report).expect("report serializes");
    rendered.push('\n');
    rendered
}

#[test]
fn inspect_json_of_rich_fixture_matches_the_golden_file() {
    let rendered = rendered_report();
    let path = golden_path();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create tests/golden");
        }
        fs::write(&path, &rendered).expect("write golden file");
        eprintln!("wrote {} ({} bytes)", path.display(), rendered.len());
        return;
    }
    let committed = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "cannot read {}: {error}\n\
             run `UPDATE_GOLDEN=1 cargo test -p hoard-gguf --test golden` to generate it",
            path.display()
        )
    });
    assert_eq!(
        committed,
        rendered,
        "{} is stale; regenerate with UPDATE_GOLDEN=1 cargo test -p hoard-gguf --test golden",
        path.display()
    );
}

#[test]
fn golden_file_is_pretty_printed_with_a_trailing_newline() {
    let rendered = rendered_report();
    assert!(rendered.ends_with("}\n"));
    assert!(rendered.starts_with("{\n  \""));
    // Two-space indentation, as produced by serde_json::to_string_pretty.
    assert!(rendered.lines().any(|line| line.starts_with("  \"file\"")));
}

#[test]
fn golden_report_is_valid_json_with_the_schema_field() {
    let rendered = rendered_report();
    let value: serde_json::Value = serde_json::from_str(&rendered).expect("golden is valid JSON");
    assert_eq!(value["schema"], 1);
    assert_eq!(value["file"]["version"], 3);
    assert_eq!(value["file"]["byte_order"], "little");
    assert_eq!(value["file"]["alignment"], 64);
    assert_eq!(value["metadata"]["count"], 24);
    assert_eq!(value["tensors"]["count"], 3);
    assert_eq!(value["tensors"]["total_bytes"], 131_496);
    assert_eq!(value["split"]["no"], 1);
    assert_eq!(value["split"]["count"], 2);
    assert_eq!(value["split"]["tensors_count"], 48);
    assert_eq!(value["warnings"], serde_json::json!([]));
    assert_eq!(value["quant_mix"][0]["type"], "F32");
    assert_eq!(value["quant_mix"][0]["percent"], 99.68);
}
