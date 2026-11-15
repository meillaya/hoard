//! Manifest v1 types and their JSON representation.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::canonical;
use crate::error::{Error, Result};
use crate::hash::{self, Blake3Digest, Hash32};
use crate::validate;

/// Version of the manifest format described by this crate.
pub const SCHEMA_VERSION: u32 = 1;

/// A hoard manifest: where an artifact set came from and the files in it.
///
/// Manifests are parsed with [`Manifest::from_json_bytes`] or
/// [`Manifest::from_path`], checked with [`Manifest::validate`], and encoded in
/// canonical form with [`Manifest::canonical_bytes`]. Parsing does not
/// validate; a parsed manifest is only trustworthy after
/// [`Manifest::validate`] returns `Ok`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Manifest format version. Manifest v1 writes `1` here.
    #[schemars(range(min = 1, max = 1))]
    pub schema: u32,

    /// Origin of the files in this manifest.
    pub source: Source,

    /// The files that make up the artifact set, sorted by `path` in canonical form.
    #[schemars(length(min = 1))]
    pub files: Vec<FileEntry>,
}

impl Manifest {
    /// Parses a manifest from JSON bytes without validating it.
    ///
    /// The input may use any valid JSON formatting and key order; call
    /// [`Manifest::validate`] afterwards to enforce the manifest v1 rules.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self> {
        Ok(serde_json::from_slice(bytes)?)
    }

    /// Reads and parses a manifest from `path` without validating it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the file cannot be read and
    /// [`Error::JsonFile`] when its contents are not a manifest.
    pub fn from_path(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|source| Error::io(path, source))?;
        serde_json::from_slice(&bytes).map_err(|source| Error::JsonFile {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Validates the manifest against every manifest v1 rule.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Validation`] describing the first rule violation, with
    /// the offending field path in the message.
    pub fn validate(&self) -> Result<()> {
        validate::validate(self)
    }

    /// Returns the canonical JSON encoding of the manifest.
    ///
    /// The bytes are UTF-8 with LF line endings and exactly one trailing
    /// newline. Files are sorted by path and sibling lists are sorted, so this
    /// canonicalises even a manifest parsed from non-canonical JSON.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonical::canonical_bytes(self)
    }

    /// Returns the BLAKE3 digest of [`Manifest::canonical_bytes`].
    pub fn hash(&self) -> Blake3Digest {
        Blake3Digest::from_hash32(hash::blake3_bytes(&self.canonical_bytes()))
    }
}

/// Where a manifest's files came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// Source kind. Manifest v1 only supports `"huggingface"`.
    pub kind: SourceKind,

    /// Hugging Face repository slug, `owner/name`.
    #[schemars(pattern(r"^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$"))]
    pub repo: String,

    /// Hugging Face repository type.
    #[serde(rename = "repoType")]
    pub repo_type: RepoType,

    /// Hugging Face commit sha: 40 lowercase hexadecimal characters.
    #[schemars(pattern(r"^[0-9a-f]{40}$"))]
    pub revision: String,
}

/// The kind of source a manifest was created from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SourceKind {
    /// A Hugging Face Hub repository.
    #[serde(rename = "huggingface")]
    HuggingFace,
}

/// The Hugging Face repository type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RepoType {
    /// A model repository.
    #[serde(rename = "model")]
    Model,
    /// A dataset repository.
    #[serde(rename = "dataset")]
    Dataset,
    /// A Space repository.
    #[serde(rename = "space")]
    Space,
}

/// One file in a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    /// POSIX-style relative path inside the artifact set, for example
    /// `tinyllamas/stories260K.gguf`.
    ///
    /// Must be non-empty, at most 1024 bytes, use `/` separators, and must not
    /// be absolute or contain `.`, `..`, or empty segments.
    #[schemars(length(max = 1024))]
    pub path: String,

    /// What the file is for.
    pub role: Role,

    /// Optional grouping key shared by related shards; allowed on `weights`,
    /// `mmproj`, and `mtp` files only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(r"^[a-z0-9][a-z0-9._-]{0,63}$"))]
    pub group: Option<String>,

    /// File size in bytes, which must be greater than zero.
    #[schemars(range(min = 1))]
    pub size: u64,

    /// SHA-256 of the file contents, as published by the source (the Hugging
    /// Face LFS oid).
    pub sha256: Hash32,

    /// BLAKE3 of the file contents: the hoard content-addressed-storage digest.
    pub blake3: Hash32,

    /// Other weights files this file ships with, for example other shards.
    ///
    /// Only allowed on `weights` files. References must be sorted and must not
    /// include the file itself; every reference must exist in `files`, point at
    /// a `weights` file in the same group, and be symmetric.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_siblings"
    )]
    pub siblings: Vec<String>,
}

/// The role of a file in an artifact set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Role {
    /// Model weights; the file a runtime actually loads.
    #[serde(rename = "weights")]
    Weights,
    /// Multimodal projection weights paired with a model.
    #[serde(rename = "mmproj")]
    Mmproj,
    /// Multi-token-prediction (draft) weights.
    #[serde(rename = "mtp")]
    Mtp,
    /// Tokenizer definitions.
    #[serde(rename = "tokenizer")]
    Tokenizer,
    /// Model configuration.
    #[serde(rename = "config")]
    Config,
    /// Any other supporting file.
    #[serde(rename = "other")]
    Other,
}

/// Deserialises `siblings`, rejecting an explicit empty array.
///
/// The field is optional but must be non-empty when present; omitting the field
/// is the way to say "no siblings".
fn deserialize_siblings<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let siblings = Vec::<String>::deserialize(deserializer)?;
    if siblings.is_empty() {
        return Err(serde::de::Error::custom(
            "siblings must not be an empty array; omit the field instead",
        ));
    }
    Ok(siblings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_json_names_match_contract() {
        let roles = [
            (Role::Weights, "weights"),
            (Role::Mmproj, "mmproj"),
            (Role::Mtp, "mtp"),
            (Role::Tokenizer, "tokenizer"),
            (Role::Config, "config"),
            (Role::Other, "other"),
        ];
        for (role, name) in roles {
            let json = serde_json::to_value(role).unwrap();
            assert_eq!(json, serde_json::json!(name));
            let parsed: Role = serde_json::from_value(json).unwrap();
            assert_eq!(parsed, role);
        }
        assert!(serde_json::from_value::<Role>(serde_json::json!("Weight")).is_err());

        let repo_types = [
            (RepoType::Model, "model"),
            (RepoType::Dataset, "dataset"),
            (RepoType::Space, "space"),
        ];
        for (repo_type, name) in repo_types {
            let json = serde_json::to_value(repo_type).unwrap();
            assert_eq!(json, serde_json::json!(name));
            let parsed: RepoType = serde_json::from_value(json).unwrap();
            assert_eq!(parsed, repo_type);
        }

        let source_kind = serde_json::to_value(SourceKind::HuggingFace).unwrap();
        assert_eq!(source_kind, serde_json::json!("huggingface"));
        assert!(serde_json::from_value::<SourceKind>(serde_json::json!("local")).is_err());
    }

    #[test]
    fn source_uses_repo_type_key() {
        let source = Source {
            kind: SourceKind::HuggingFace,
            repo: "ggml-org/models".to_owned(),
            repo_type: RepoType::Model,
            revision: "499bc8821c6b12b4e53c5bffcb21ec206f212d81".to_owned(),
        };
        let json = serde_json::to_value(&source).unwrap();
        assert_eq!(json["repoType"], serde_json::json!("model"));
        assert!(json.get("repo_type").is_none());
        let parsed: Source = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, source);
    }

    #[test]
    fn file_entry_omits_empty_optional_fields() {
        let entry = FileEntry {
            path: "weights/model.gguf".to_owned(),
            role: Role::Weights,
            group: None,
            size: 1,
            sha256: Hash32::from_bytes([1; 32]),
            blake3: Hash32::from_bytes([2; 32]),
            siblings: Vec::new(),
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert!(json.get("group").is_none());
        assert!(json.get("siblings").is_none());

        let parsed: FileEntry = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn siblings_rejects_empty_array() {
        let json = serde_json::json!({
            "path": "weights/model.gguf",
            "role": "weights",
            "size": 1,
            "sha256": "0000000000000000000000000000000000000000000000000000000000000001",
            "blake3": "0000000000000000000000000000000000000000000000000000000000000002",
            "siblings": []
        });
        let error = serde_json::from_value::<FileEntry>(json).unwrap_err();
        assert!(error.to_string().contains("siblings"), "{error}");
    }
}
