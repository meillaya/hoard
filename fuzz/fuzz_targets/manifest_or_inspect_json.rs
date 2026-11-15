#![no_main]

//! Fuzz target: serialize the frozen inspect JSON for every valid parse.
//!
//! Contract under test: a file accepted by `Gguf::parse` must always produce an
//! inspect report that serializes to valid JSON and round-trips to an
//! equivalent document, with `schema: 1`. The committed JSON Schema is
//! generated from the same types, so it must stay constructible too.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(gguf) = hoard_gguf::Gguf::parse(data) else {
        return;
    };
    let report = hoard_gguf::inspect(&gguf, Some(std::path::Path::new("fuzz.gguf")));

    // Pretty JSON is what the CLI prints for --json: two-space indentation and
    // a trailing newline (the CLI appends it; the document itself is valid).
    let pretty = serde_json::to_string_pretty(&report)
        .expect("inspect report must serialize as pretty JSON");
    let value: serde_json::Value =
        serde_json::from_str(&pretty).expect("pretty inspect JSON must parse");
    assert_eq!(value["schema"], 1, "inspect schema version must be 1");

    // Compact JSON must describe exactly the same document.
    let compact =
        serde_json::to_string(&report).expect("inspect report must serialize as compact JSON");
    let compact_value: serde_json::Value =
        serde_json::from_str(&compact).expect("compact inspect JSON must parse");
    assert_eq!(
        compact_value, value,
        "pretty and compact inspect JSON must be equivalent"
    );

    // The JSON Schema for the report is derived from the same Rust types.
    let schema = hoard_gguf::inspect_schema();
    assert_eq!(
        schema["type"], "object",
        "inspect schema root must be an object"
    );
});
