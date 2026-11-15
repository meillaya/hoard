//! JSON Schema generation for manifest v1.

use crate::canonical::sort_object_keys;
use crate::manifest::Manifest;

/// Returns the JSON Schema for manifest v1, generated from the [`Manifest`]
/// types.
///
/// The schema is deterministic: object keys are sorted lexicographically, so it
/// can be committed to `schemas/manifest-v1.schema.json` and regenerated with
/// `UPDATE_SCHEMAS=1 cargo test -p hoard-core --test schema`.
pub fn manifest_schema() -> serde_json::Value {
    let schema = schemars::schema_for!(Manifest);
    sort_object_keys(schema.to_value())
}
