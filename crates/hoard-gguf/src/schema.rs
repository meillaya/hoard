//! JSON Schema generation for the `inspect` report.
//!
//! The schema is generated from [`InspectReport`] with `schemars`, so it can
//! never drift from the Rust types. It is committed at
//! `schemas/inspect-v1.schema.json`; the unit test at the bottom of this module
//! compares the committed file with the generated one and can rewrite it with
//! `UPDATE_SCHEMAS=1`.

use crate::inspect::InspectReport;

/// Generate the JSON Schema for the version 1 `inspect` report.
///
/// The returned value is the root schema object (with `$schema`, `title`,
/// `type`, and `properties` keys) as plain `serde_json` JSON.
#[must_use]
pub fn inspect_schema() -> serde_json::Value {
    schemars::schema_for!(InspectReport).to_value()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn schema_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/inspect-v1.schema.json")
    }

    fn render_schema() -> String {
        serde_json::to_string_pretty(&inspect_schema()).expect("schema serializes to JSON") + "\n"
    }

    #[test]
    fn committed_schema_matches_the_generated_schema() {
        let path = schema_path();
        let rendered = render_schema();
        if std::env::var_os("UPDATE_SCHEMAS").is_some() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create schemas directory");
            }
            std::fs::write(&path, rendered).expect("write schema file");
            eprintln!("updated {}", path.display());
            return;
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "cannot read {}: {error}\nrun `UPDATE_SCHEMAS=1 cargo test -p hoard-gguf --lib schema` to generate it",
                path.display()
            )
        });
        assert_eq!(
            committed,
            rendered,
            "{} is stale; regenerate with `UPDATE_SCHEMAS=1 cargo test -p hoard-gguf --lib schema`",
            path.display()
        );
    }

    #[test]
    fn schema_describes_the_inspect_report_root() {
        let schema = inspect_schema();
        let object = schema.as_object().expect("root schema is an object");
        assert_eq!(object["type"], "object");
        let properties = object["properties"]
            .as_object()
            .expect("root schema has properties");
        for field in [
            "schema",
            "file",
            "model",
            "split",
            "metadata",
            "tensors",
            "quant_mix",
            "memory",
            "warnings",
        ] {
            assert!(
                properties.contains_key(field),
                "schema is missing top-level property {field:?}"
            );
        }
        assert_eq!(properties["schema"]["type"], "integer");
        let required: Vec<&str> = object["required"]
            .as_array()
            .expect("root schema has required fields")
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
        for field in required {
            assert!(properties.contains_key(field));
        }
    }
}
