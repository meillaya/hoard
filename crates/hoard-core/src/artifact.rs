//! Manifest building from a source listing, verified CAS entries, and a
//! selection.
//!
//! The builder is the step between "which files exist at the source" and "the
//! canonical manifest of the artifact set": the caller lists the source files
//! with their published SHA-256 digests, fetches and stores the bytes, and
//! selects a subset. [`build_manifest`] joins those inputs into a validated
//! [`Manifest`] whose `blake3` fields come from the content-addressed store and
//! whose split siblings are written symmetrically. [`write_canonical`] then
//! writes the canonical bytes atomically.
//!
//! The builder never touches the network and never reads file contents: it
//! trusts `remote` for source metadata and the [`cas::Entry`] values for the
//! bytes, and it rejects any disagreement between the two.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::cas;
use crate::error::{Error, Result};
use crate::hash::Hash32;
use crate::manifest::{FileEntry, Manifest, Role, SCHEMA_VERSION, Source};

/// Length in bytes of the `-NNNNN-of-NNNNN.gguf` split suffix.
const SPLIT_SUFFIX_LEN: usize = 20;

/// A file as listed by the artifact source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// POSIX-relative path inside the source repository.
    pub path: String,

    /// File size in bytes, as published by the source.
    pub size: u64,

    /// SHA-256 digest of the file, as published by the source (the Hugging
    /// Face LFS oid).
    pub sha256: Hash32,
}

/// The files to include in the manifest that is being built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// Source paths to include, in any order.
    pub files: Vec<String>,

    /// Optional group assigned to the selected `weights` files.
    ///
    /// The group ties split shards together and must match the manifest group
    /// rules (`^[a-z0-9][a-z0-9._-]{0,63}$`). It is not written for files with
    /// any other role.
    pub group: Option<String>,

    /// Optional role for every selected file.
    ///
    /// When set it replaces [`infer_role`] for the whole selection, which is
    /// how a caller describes files whose names do not follow the usual
    /// conventions. When `None`, each role is inferred from its path.
    pub role: Option<Role>,
}

/// Returns the role inferred from a source path.
///
/// The rule looks at the base name only, in this order: `mmproj*` is
/// [`Role::Mmproj`], `mtp*` is [`Role::Mtp`], `tokenizer*` or `vocab*` is
/// [`Role::Tokenizer`], exactly `config.json` is [`Role::Config`], `*.gguf` is
/// [`Role::Weights`], and everything else is [`Role::Other`].
///
/// Matching is case-sensitive: source paths are case-sensitive and Hugging
/// Face model files use these lower-case names.
pub fn infer_role(path: &str) -> Role {
    let name = base_name(path);
    if name.starts_with("mmproj") {
        Role::Mmproj
    } else if name.starts_with("mtp") {
        Role::Mtp
    } else if name.starts_with("tokenizer") || name.starts_with("vocab") {
        Role::Tokenizer
    } else if name == "config.json" {
        Role::Config
    } else if name.ends_with(".gguf") {
        Role::Weights
    } else {
        Role::Other
    }
}

/// Returns the file name of a POSIX-style path, or the whole string when it
/// has no `/`.
fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Detects split files among `paths` and returns their sibling membership.
///
/// A split file ends in `-NNNNN-of-NNNNN.gguf` (five ASCII digits in each
/// number, for example `model-00002-of-00003.gguf`). For every split file the
/// result holds a tuple `(path, other members, total)`: the other members of
/// the same stem, sorted lexicographically and excluding the path itself, and
/// the declared total number of parts. The result is sorted by path.
///
/// Paths that do not end in the split suffix are ignored. Every detected split
/// set must be complete: all members of one stem must declare the same total,
/// part numbers must be unique and in `1..=total`, and every part
/// `1..=total` must be present. A set that is missing a part is an error, so a
/// manifest never records a partial split relation.
///
/// # Errors
///
/// Returns [`Error::Validation`] naming the offending paths when the totals are
/// inconsistent, a part number is out of range or duplicated, or a member is
/// missing.
pub fn split_members(paths: &[String]) -> Result<Vec<(String, Vec<String>, u32)>> {
    let mut sets: BTreeMap<&str, Vec<SplitName<'_>>> = BTreeMap::new();
    for path in paths {
        if let Some(split) = parse_split_name(path) {
            sets.entry(split.stem).or_default().push(split);
        }
    }

    let mut members = Vec::new();
    for (stem, mut names) in sets {
        let total = names[0].total;
        if let Some(other) = names.iter().find(|name| name.total != total) {
            return Err(Error::Validation(format!(
                "artifact: split set {stem:?}: {} declares of-{:05} but {} declares of-{:05}; \
                 all members must declare the same total",
                names[0].path, total, other.path, other.total
            )));
        }

        names.sort_by_key(|name| name.number);
        for name in &names {
            if name.number == 0 || name.number > total {
                return Err(Error::Validation(format!(
                    "artifact: split set {stem:?}: {} declares part {} of {}; \
                     part numbers must be in 1..={total}",
                    name.path, name.number, total
                )));
            }
        }
        for pair in names.windows(2) {
            if pair[0].number == pair[1].number {
                return Err(Error::Validation(format!(
                    "artifact: split set {stem:?}: {} and {} both declare part {:05}",
                    pair[0].path, pair[1].path, pair[0].number
                )));
            }
        }

        if names.len() as u32 != total {
            let present: BTreeSet<u32> = names.iter().map(|name| name.number).collect();
            let missing_count = total - names.len() as u32;
            let missing: Vec<String> = (1..=total)
                .filter(|number| !present.contains(number))
                .take(5)
                .map(|number| format!("{stem}-{number:05}-of-{total:05}.gguf"))
                .collect();
            let more = if missing_count as usize > missing.len() {
                format!(" and {} more", missing_count as usize - missing.len())
            } else {
                String::new()
            };
            return Err(Error::Validation(format!(
                "artifact: split set {stem:?} declares {total} parts but {} member(s) were given; \
                 missing: {}{more}",
                names.len(),
                missing.join(", ")
            )));
        }

        let sorted_paths: Vec<String> = names.iter().map(|name| name.path.to_owned()).collect();
        for name in &names {
            let siblings: Vec<String> = sorted_paths
                .iter()
                .filter(|path| path.as_str() != name.path)
                .cloned()
                .collect();
            members.push((name.path.to_owned(), siblings, total));
        }
    }

    members.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(members)
}

/// A parsed split file name.
struct SplitName<'a> {
    /// The full path.
    path: &'a str,
    /// The path with the split suffix removed; the split set key.
    stem: &'a str,
    /// The part number, `N` in `-NNNNN-of-NNNNN.gguf`.
    number: u32,
    /// The declared total number of parts, `M` in `-NNNNN-of-NNNNN.gguf`.
    total: u32,
}

/// Parses `-NNNNN-of-NNNNN.gguf` at the end of `path`, returning `None` when
/// the path does not have that suffix.
fn parse_split_name(path: &str) -> Option<SplitName<'_>> {
    if path.len() < SPLIT_SUFFIX_LEN {
        return None;
    }

    let suffix_start = path.len() - SPLIT_SUFFIX_LEN;
    let suffix = &path.as_bytes()[suffix_start..];
    let matches_suffix = suffix[0] == b'-'
        && suffix[6] == b'-'
        && suffix[7] == b'o'
        && suffix[8] == b'f'
        && suffix[9] == b'-'
        && suffix[15] == b'.'
        && &suffix[16..] == b"gguf";
    if !matches_suffix {
        return None;
    }

    Some(SplitName {
        path,
        stem: &path[..suffix_start],
        number: parse_split_number(&suffix[1..6])?,
        total: parse_split_number(&suffix[10..15])?,
    })
}

/// Parses exactly five ASCII digits as a `u32`.
fn parse_split_number(bytes: &[u8]) -> Option<u32> {
    if !bytes.iter().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

/// Builds and validates a manifest from the source listing, the fetched CAS
/// entries, and the selection.
///
/// The builder joins `selection.files` with `remote` and `fetched`:
///
/// - every selected path must be listed in `remote`, otherwise the error lists
///   the available files;
/// - every selected path must have an entry in `fetched`, and the entry must
///   agree with `remote` on size and SHA-256;
/// - the role is [`Selection::role`] when set and [`infer_role`] otherwise;
/// - `Selection::group` is written on the selected `weights` files only;
/// - split weights are detected with [`split_members`] and every member gets
///   the sorted, self-excluding list of its siblings; files with other roles
///   never get siblings;
/// - `size` and `sha256` come from `remote`, `blake3` from the CAS entry;
/// - the files are sorted by path and the result is validated with
///   [`Manifest::validate`].
///
/// Fetched entries that are not selected are ignored.
///
/// # Errors
///
/// Returns [`Error::Validation`] with the offending path when a selected path
/// is missing from `remote` or `fetched`, when source metadata and CAS entry
/// disagree, when the selection is empty or contains duplicates, when a split
/// set is incomplete or inconsistent, or when the built manifest fails
/// [`Manifest::validate`]. Returns [`Error::Io`] only through hashing helpers;
/// the builder itself reads no files.
pub fn build_manifest(
    source: Source,
    remote: &[SourceFile],
    fetched: &[(String, cas::Entry)],
    selection: &Selection,
) -> Result<Manifest> {
    if selection.files.is_empty() {
        return Err(Error::Validation(
            "artifact: selection.files must contain at least one path".to_owned(),
        ));
    }

    let remote_by_path: BTreeMap<&str, &SourceFile> = remote
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    let fetched_by_path: BTreeMap<&str, &cas::Entry> = fetched
        .iter()
        .map(|(path, entry)| (path.as_str(), entry))
        .collect();

    let mut seen = BTreeSet::new();
    let mut resolved: Vec<(&SourceFile, Role, &cas::Entry)> =
        Vec::with_capacity(selection.files.len());
    for path in &selection.files {
        if !seen.insert(path.as_str()) {
            return Err(Error::Validation(format!(
                "artifact: selection.files lists {path:?} more than once"
            )));
        }

        let remote_file = remote_by_path.get(path.as_str()).ok_or_else(|| {
            Error::Validation(format!(
                "artifact: selection path {path:?} is not in the source listing; \
                 available files: {}",
                listed_paths(remote.iter().map(|file| file.path.as_str()))
            ))
        })?;
        let entry = fetched_by_path.get(path.as_str()).ok_or_else(|| {
            Error::Validation(format!(
                "artifact: selection path {path:?} has no content-addressed entry; \
                 fetched files: {}",
                listed_paths(fetched.iter().map(|(path, _)| path.as_str()))
            ))
        })?;

        if remote_file.size == 0 {
            return Err(Error::Validation(format!(
                "artifact: {path:?} has size 0 in the source listing; \
                 every manifest file must be larger than zero"
            )));
        }
        if entry.size != remote_file.size {
            return Err(Error::Validation(format!(
                "artifact: {path:?} has size {} in the source listing but {} in its \
                 content-addressed entry",
                remote_file.size, entry.size
            )));
        }
        if entry.sha256 != remote_file.sha256 {
            return Err(Error::Validation(format!(
                "artifact: {path:?} has sha256 {} in the source listing but {} in its \
                 content-addressed entry",
                remote_file.sha256, entry.sha256
            )));
        }

        let role = selection.role.unwrap_or_else(|| infer_role(path));
        resolved.push((remote_file, role, entry));
    }

    let weight_paths: Vec<String> = resolved
        .iter()
        .filter(|(_, role, _)| *role == Role::Weights)
        .map(|(file, _, _)| file.path.clone())
        .collect();
    let mut siblings_by_path: BTreeMap<String, Vec<String>> = split_members(&weight_paths)?
        .into_iter()
        .map(|(path, siblings, _total)| (path, siblings))
        .collect();

    let mut files: Vec<FileEntry> = resolved
        .iter()
        .map(|(remote_file, role, entry)| FileEntry {
            path: remote_file.path.clone(),
            role: *role,
            group: if *role == Role::Weights {
                selection.group.clone()
            } else {
                None
            },
            size: remote_file.size,
            sha256: remote_file.sha256,
            blake3: entry.blake3,
            siblings: siblings_by_path
                .remove(&remote_file.path)
                .unwrap_or_default(),
        })
        .collect();
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let manifest = Manifest {
        schema: SCHEMA_VERSION,
        source,
        files,
    };
    manifest
        .validate()
        .map_err(|error| attribute_validation_error(&manifest, error))?;

    Ok(manifest)
}

/// Writes the canonical encoding of `manifest` to `path` atomically.
///
/// The bytes are written to a temporary file in the directory that will hold
/// `path` and renamed over `path`, so a reader never observes a partial
/// manifest. An existing file at `path` is replaced and the parent directory
/// must already exist.
///
/// # Errors
///
/// Returns [`Error::Io`] when the temporary file cannot be written, fsynced, or
/// renamed.
pub fn write_canonical(manifest: &Manifest, path: &Path) -> Result<()> {
    let bytes = manifest.canonical_bytes();
    let parent = parent_directory(path);
    let mut temp =
        tempfile::NamedTempFile::new_in(&parent).map_err(|source| Error::io(path, source))?;
    temp.write_all(&bytes)
        .map_err(|source| Error::io(path, source))?;
    temp.as_file_mut()
        .flush()
        .map_err(|source| Error::io(path, source))?;
    temp.as_file()
        .sync_all()
        .map_err(|source| Error::io(path, source))?;
    temp.persist(path)
        .map_err(|error| Error::io(path, error.error))?;

    Ok(())
}

/// Formats a list of paths for an error message.
fn listed_paths<'a>(paths: impl Iterator<Item = &'a str>) -> String {
    let paths: Vec<&str> = paths.collect();
    if paths.is_empty() {
        return "none".to_owned();
    }
    paths
        .iter()
        .map(|path| format!("{path:?}"))
        .collect::<Vec<String>>()
        .join(", ")
}

/// Returns the directory that should hold a temporary file for `path`, using
/// the current directory for a bare file name.
fn parent_directory(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Adds the offending file path to a manifest validation error when possible.
///
/// The manifest is built from a selection, so a rule violation belongs to one
/// of its files in practice. The whole-manifest error is returned unchanged
/// when the failure comes from the schema or the source, or when no single
/// file reproduces it.
fn attribute_validation_error(manifest: &Manifest, error: Error) -> Error {
    let detail = match &error {
        Error::Validation(detail) => detail.clone(),
        _ => return error,
    };

    let source_and_schema_ok = Manifest {
        schema: SCHEMA_VERSION,
        source: manifest.source.clone(),
        files: vec![probe_file()],
    }
    .validate()
    .is_ok();
    if !source_and_schema_ok {
        return error;
    }

    for file in &manifest.files {
        let mut probe = file.clone();
        probe.siblings.clear();
        let probe_manifest = Manifest {
            schema: SCHEMA_VERSION,
            source: manifest.source.clone(),
            files: vec![probe],
        };
        if probe_manifest.validate().is_err() {
            return Error::Validation(format!("{}: {detail}", file.path));
        }
    }

    error
}

/// A minimal manifest file used to isolate source and schema errors.
fn probe_file() -> FileEntry {
    FileEntry {
        path: "probe.gguf".to_owned(),
        role: Role::Weights,
        group: None,
        size: 1,
        sha256: Hash32::from_bytes([0; 32]),
        blake3: Hash32::from_bytes([0; 32]),
        siblings: Vec::new(),
    }
}
