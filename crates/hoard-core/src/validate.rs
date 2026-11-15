//! Manifest v1 validation rules.

use std::collections::BTreeMap;

use crate::error::{Error, Result};
use crate::manifest::{FileEntry, Manifest, Role, SCHEMA_VERSION, Source};

/// Maximum path length in bytes.
const MAX_PATH_BYTES: usize = 1024;

/// Length of a Hugging Face commit sha, in lowercase hexadecimal characters.
const REVISION_LEN: usize = 40;

/// Maximum length of a `group` string in characters.
const MAX_GROUP_CHARS: usize = 64;

/// Validates `manifest` against every manifest v1 rule.
///
/// Checks the schema version, the source repository slug and revision, that
/// `files` is non-empty, that every path is a safe POSIX relative path, that
/// role/group/sibling rules hold, that files are sorted by path with no
/// duplicates, and that sibling references exist, point at `weights` files in
/// the same group, and are symmetric.
pub fn validate(manifest: &Manifest) -> Result<()> {
    if manifest.schema != SCHEMA_VERSION {
        return Err(Error::Validation(format!(
            "schema: expected {SCHEMA_VERSION}, found {}",
            manifest.schema
        )));
    }

    validate_source(&manifest.source)?;

    if manifest.files.is_empty() {
        return Err(Error::Validation(
            "files: must contain at least one file".to_owned(),
        ));
    }

    let mut by_path: BTreeMap<&str, &FileEntry> = BTreeMap::new();
    for (index, file) in manifest.files.iter().enumerate() {
        let context = format!("files[{index}]");
        validate_path(&file.path)
            .map_err(|message| Error::Validation(format!("{context}.path: {message}")))?;
        validate_role_and_group(file, &context)?;

        if file.size == 0 {
            return Err(Error::Validation(format!(
                "{context}.size: must be greater than zero"
            )));
        }

        if !file.siblings.is_empty() {
            validate_siblings_sorted_and_self_excluding(file, &context)?;
        }

        if by_path.insert(&file.path, file).is_some() {
            return Err(Error::Validation(format!(
                "{context}.path: duplicate path {:?}; every path must be unique",
                file.path
            )));
        }
    }

    for pair in manifest.files.windows(2) {
        if pair[0].path >= pair[1].path {
            return Err(Error::Validation(format!(
                "files: must be sorted by path: {:?} appears before {:?}",
                pair[0].path, pair[1].path
            )));
        }
    }

    for (index, file) in manifest.files.iter().enumerate() {
        for (sibling_index, sibling) in file.siblings.iter().enumerate() {
            let context = format!("files[{index}].siblings[{sibling_index}]");
            let referenced = by_path.get(sibling.as_str()).ok_or_else(|| {
                Error::Validation(format!("{context}: {sibling:?} is not listed in files"))
            })?;
            if referenced.role != Role::Weights {
                return Err(Error::Validation(format!(
                    "{context}: {sibling:?} has role {:?}; siblings must reference weights files",
                    referenced.role
                )));
            }
            if referenced.group != file.group {
                return Err(Error::Validation(format!(
                    "{context}: {sibling:?} must have the same group as {:?}",
                    file.path
                )));
            }
            if !referenced.siblings.iter().any(|path| path == &file.path) {
                return Err(Error::Validation(format!(
                    "{context}: sibling relation must be symmetric; {sibling:?} does not list {:?}",
                    file.path
                )));
            }
        }
    }

    Ok(())
}

/// Validates the `source` object.
fn validate_source(source: &Source) -> Result<()> {
    validate_repo(&source.repo)
        .map_err(|message| Error::Validation(format!("source.repo: {message}")))?;

    if source.revision.len() != REVISION_LEN || !source.revision.bytes().all(is_lowercase_hex) {
        return Err(Error::Validation(format!(
            "source.revision: must be the {REVISION_LEN}-character lowercase hexadecimal \
             Hugging Face commit sha, found {:?}",
            source.revision
        )));
    }

    Ok(())
}

/// Validates a Hugging Face `owner/name` repository slug.
fn validate_repo(repo: &str) -> std::result::Result<(), String> {
    if repo.contains("..") {
        return Err("must not contain '..'".to_owned());
    }

    let mut parts = repo.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if parts.next().is_some() {
        return Err("must be an 'owner/name' slug with exactly one '/'".to_owned());
    }
    if owner.is_empty() || name.is_empty() {
        return Err("owner and name must both be non-empty".to_owned());
    }

    for part in [owner, name] {
        if !part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(format!(
                "{part:?} may only contain ASCII letters, digits, '.', '_', and '-'"
            ));
        }
        let bytes = part.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if matches!(first, b'-' | b'.') || matches!(last, b'-' | b'.') {
            return Err(format!("{part:?} must not start or end with '-' or '.'"));
        }
    }

    Ok(())
}

/// Validates a POSIX-style relative path.
fn validate_path(path: &str) -> std::result::Result<(), String> {
    if path.is_empty() {
        return Err("must not be empty".to_owned());
    }
    if path.len() > MAX_PATH_BYTES {
        return Err(format!(
            "must be at most {MAX_PATH_BYTES} bytes, found {}",
            path.len()
        ));
    }
    if path.starts_with('/') {
        return Err("must be relative (must not start with '/')".to_owned());
    }
    if path.contains('\\') {
        return Err("must use POSIX separators (must not contain '\\')".to_owned());
    }
    if path.contains('\0') {
        return Err("must not contain a NUL character".to_owned());
    }
    for segment in path.split('/') {
        if segment.is_empty() {
            return Err("must not contain empty path segments".to_owned());
        }
        if matches!(segment, "." | "..") {
            return Err("must not contain '.' or '..' path segments".to_owned());
        }
    }

    Ok(())
}

/// Validates the role/group/siblings relationship of a file.
fn validate_role_and_group(file: &FileEntry, context: &str) -> Result<()> {
    let supports_group = matches!(file.role, Role::Weights | Role::Mmproj | Role::Mtp);
    if let Some(group) = &file.group {
        if !supports_group {
            return Err(Error::Validation(format!(
                "{context}.group: not allowed for role {:?}",
                file.role
            )));
        }
        validate_group(group)
            .map_err(|message| Error::Validation(format!("{context}.group: {message}")))?;
    }

    if !file.siblings.is_empty() && file.role != Role::Weights {
        return Err(Error::Validation(format!(
            "{context}.siblings: only allowed on weights files, not role {:?}",
            file.role
        )));
    }

    Ok(())
}

/// Validates a `group` string: `^[a-z0-9][a-z0-9._-]{0,63}$`.
fn validate_group(group: &str) -> std::result::Result<(), String> {
    let mut characters = group.chars();
    let Some(first) = characters.next() else {
        return Err("must not be empty".to_owned());
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err("must start with a lowercase ASCII letter or digit".to_owned());
    }
    if group.chars().count() > MAX_GROUP_CHARS {
        return Err(format!(
            "must be at most {MAX_GROUP_CHARS} characters, found {}",
            group.chars().count()
        ));
    }
    if !characters.all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '.' | '_' | '-')
    }) {
        return Err(
            "may only contain lowercase ASCII letters, digits, '.', '_', and '-'".to_owned(),
        );
    }

    Ok(())
}

/// Validates sibling syntax and ordering; resolution happens in [`validate`].
fn validate_siblings_sorted_and_self_excluding(file: &FileEntry, context: &str) -> Result<()> {
    for (index, sibling) in file.siblings.iter().enumerate() {
        let sibling_context = format!("{context}.siblings[{index}]");
        validate_path(sibling)
            .map_err(|message| Error::Validation(format!("{sibling_context}: {message}")))?;
        if sibling == &file.path {
            return Err(Error::Validation(format!(
                "{sibling_context}: a file must not list itself as a sibling"
            )));
        }
    }

    for pair in file.siblings.windows(2) {
        if pair[0] >= pair[1] {
            return Err(Error::Validation(format!(
                "{context}.siblings: must be sorted lexicographically with no duplicates: \
                 {:?} does not come before {:?}",
                pair[0], pair[1]
            )));
        }
    }

    Ok(())
}

/// Returns whether `byte` is a lowercase hexadecimal digit.
fn is_lowercase_hex(byte: u8) -> bool {
    matches!(byte, b'0'..=b'9' | b'a'..=b'f')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_safe_paths() {
        for path in [
            "model.gguf",
            "tinyllamas/stories260K.gguf",
            "a/b/c.txt",
            "dir/模型.gguf",
            "a..b/c",
            ".hidden/file",
        ] {
            assert!(validate_path(path).is_ok(), "{path:?}");
        }
    }

    #[test]
    fn rejects_unsafe_paths() {
        let long = "a".repeat(MAX_PATH_BYTES + 1);
        for path in [
            "",
            "/absolute/path",
            "trailing/",
            "a//b",
            "./a",
            "a/./b",
            "../a",
            "a/../b",
            "a\\b",
            "a\0b",
            long.as_str(),
        ] {
            assert!(validate_path(path).is_err(), "{path:?}");
        }
        assert!(validate_path(&"a".repeat(MAX_PATH_BYTES)).is_ok());
    }

    #[test]
    fn accepts_repo_slugs() {
        for repo in ["owner/model", "ggml-org/models", "a/b", "A.B-C_d/e.f"] {
            assert!(validate_repo(repo).is_ok(), "{repo:?}");
        }
    }

    #[test]
    fn rejects_bad_repo_slugs() {
        for repo in [
            "",
            "owner",
            "owner/",
            "/model",
            "a/b/c",
            "own..er/model",
            "owner/..",
            "-owner/model",
            "owner./model",
            ".owner/model",
            "owner/.model",
            "owner name/model",
            "ownér/model",
        ] {
            assert!(validate_repo(repo).is_err(), "{repo:?}");
        }
    }

    #[test]
    fn accepts_valid_groups() {
        for group in ["a", "0", "stories", "stories-260k", "q4_k_m", "a.b_c-d"] {
            assert!(validate_group(group).is_ok(), "{group:?}");
        }
        assert!(validate_group(&"a".repeat(MAX_GROUP_CHARS)).is_ok());
    }

    #[test]
    fn rejects_bad_groups() {
        let long = format!("a{}", "b".repeat(MAX_GROUP_CHARS));
        for group in ["", "-a", ".a", "_a", "A", "aB", "a b", "a/b", long.as_str()] {
            assert!(validate_group(group).is_err(), "{group:?}");
        }
    }
}
