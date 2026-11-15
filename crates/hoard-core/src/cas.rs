//! Content-addressed storage for verified artifact bytes.
//!
//! The store keeps blobs under `<root>/blobs/<first two hex>/<64 hex>`, named
//! by their BLAKE3 digest, and writes through `<root>/tmp`. Adding a file
//! computes the source SHA-256 and the BLAKE3 in one streaming pass while
//! copying it, checks the expected SHA-256, and renames a fsynced temporary
//! file into place, so a killed process never leaves a partial blob at its
//! final path. Reading re-hashes the blob: a corrupt or attacker-written entry
//! is rejected instead of used.
//!
//! Filesystem failures are returned as [`Error::Io`]. Semantic failures (a
//! digest mismatch, a corrupt blob, a refused destination) are returned as
//! [`Error::Validation`] with the offending path in the message.
//!
//! # Layout
//!
//! ```text
//! <root>/blobs/<first two hex>/<64 lowercase hex>   stored blob
//! <root>/tmp/                                        in-progress writes
//! ```
//!
//! The store holds no index: a file's BLAKE3 digest is its identity and its
//! path, so two processes can populate it concurrently. Writes are atomic per
//! blob; readers must still call [`Store::verify`] (or [`Store::materialize`],
//! which verifies) before trusting bytes on disk.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use blake3::Hasher as Blake3Hasher;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::hash::Hash32;

/// Number of bytes read from a file per hashing or copy step.
const BUFFER_SIZE: usize = 1024 * 1024;

/// Name of the directory under the store root that holds the blobs.
const BLOBS_DIR: &str = "blobs";

/// Name of the directory under the store root that holds in-progress writes.
const TMP_DIR: &str = "tmp";

/// A content-addressed store rooted at a directory.
///
/// A store is a plain directory tree with no index. A blob's identity is its
/// BLAKE3 digest, which is also its path, so the layout is stable across runs
/// and processes. Use [`Store::open`] to create the layout, [`Store::put_file`]
/// to add a verified file, [`Store::verify`] to re-hash an entry, and
/// [`Store::materialize`] to copy an entry out of the store.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

/// The identity of one stored file.
///
/// An entry is always built from bytes that hoard hashed itself:
/// [`Store::put_file`] only returns it after checking the source SHA-256 and
/// computing the BLAKE3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// BLAKE3 digest of the stored bytes: the store identity and blob name.
    pub blake3: Hash32,

    /// SHA-256 digest of the stored bytes, as published by the source.
    pub sha256: Hash32,

    /// Size of the stored bytes, in bytes.
    pub size: u64,
}

impl Store {
    /// Opens the store rooted at `root`, creating the `blobs/` and `tmp/`
    /// directories when they do not exist.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when a directory cannot be created.
    pub fn open(root: PathBuf) -> Result<Self> {
        let store = Self { root };
        store.ensure_directories()?;
        Ok(store)
    }

    /// Returns the root directory of the store.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the blob path for a BLAKE3 digest:
    /// `<root>/blobs/<first two hex>/<64 lowercase hex>`.
    ///
    /// The path is derived from digest bytes only, so it always stays inside
    /// `<root>/blobs`; it does not have to exist.
    pub fn path_for(&self, blake3: &Hash32) -> PathBuf {
        let hex = blake3.to_hex();
        self.root.join(BLOBS_DIR).join(&hex[..2]).join(&hex)
    }

    /// Returns whether a blob with this digest exists.
    ///
    /// This is a cheap existence check: it does not hash the file, so a
    /// poisoned blob also reports `true`. Call [`Store::verify`] before using
    /// the bytes.
    pub fn contains(&self, blake3: &Hash32) -> bool {
        self.path_for(blake3).is_file()
    }

    /// Adds the file at `path` to the store and returns its entry.
    ///
    /// The file is read twice when a new blob is written. The first pass
    /// streams the SHA-256 and the BLAKE3 in one go and checks that the SHA-256
    /// matches `expected_sha256`; the second pass copies the file into a
    /// temporary file under `<root>/tmp` while re-hashing it, rejects the copy
    /// if the source changed between the passes, fsyncs the temporary file, and
    /// renames it to the blob path. A failed put leaves no blob behind.
    ///
    /// When a blob with the same BLAKE3 digest already exists and verifies, it
    /// is kept and its entry is returned without copying the file again.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Validation`] when the SHA-256 does not match
    /// `expected_sha256`, when the source changes while it is stored, or when
    /// the existing blob does not verify; returns [`Error::Io`] when the source
    /// cannot be read or the blob cannot be written.
    pub fn put_file(&self, path: &Path, expected_sha256: Hash32) -> Result<Entry> {
        let (sha256, blake3, size) = hash_file(path)?;
        if sha256 != expected_sha256 {
            return Err(Error::Validation(format!(
                "content-addressed store: sha256 mismatch for {}: expected {expected_sha256}, computed {sha256}",
                path.display()
            )));
        }

        let entry = Entry {
            blake3,
            sha256,
            size,
        };
        let blob = self.path_for(&entry.blake3);
        if self.verify(&entry).is_ok() {
            return Ok(entry);
        }

        self.write_blob(path, &entry, &blob)?;
        Ok(entry)
    }

    /// Re-hashes the blob for `entry` and checks its size, BLAKE3, and SHA-256.
    ///
    /// This is what makes a poisoned store safe to read: the digest is
    /// recomputed from the bytes on disk and compared with the entry, never
    /// trusted from the file name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Validation`] with the blob path when the size or either
    /// digest differs from `entry`; returns [`Error::Io`] when the blob cannot
    /// be read.
    pub fn verify(&self, entry: &Entry) -> Result<()> {
        let blob = self.path_for(&entry.blake3);
        let (sha256, blake3, size) = hash_file(&blob)?;

        if size != entry.size {
            return Err(Error::Validation(format!(
                "content-addressed store: corrupt blob {}: expected {} bytes, found {size}",
                blob.display(),
                entry.size
            )));
        }
        if blake3 != entry.blake3 {
            return Err(Error::Validation(format!(
                "content-addressed store: corrupt blob {}: expected blake3 {}, computed {blake3}",
                blob.display(),
                entry.blake3
            )));
        }
        if sha256 != entry.sha256 {
            return Err(Error::Validation(format!(
                "content-addressed store: corrupt blob {}: expected sha256 {}, computed {sha256}",
                blob.display(),
                entry.sha256
            )));
        }

        Ok(())
    }

    /// Verifies `entry`, then copies its blob to `dest` with mode `0644`.
    ///
    /// Missing parent directories of `dest` are created with mode `0755`
    /// (subject to the process umask when the tree is created, the created
    /// directories are set explicitly). The copy is written to a temporary file
    /// in the destination directory and renamed over `dest`, so `dest` is never
    /// a partial file. An existing regular file at `dest` is replaced.
    ///
    /// The caller owns the destination root; this function does not police
    /// where `dest` points. It does refuse to write through a symlink at `dest`
    /// itself and refuses a directory at `dest`.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Store::verify`] first, then
    /// [`Error::Validation`] when `dest` is a symlink or a directory, and
    /// [`Error::Io`] when the copy or rename fails.
    pub fn materialize(&self, entry: &Entry, dest: &Path) -> Result<()> {
        self.verify(entry)?;

        refuse_non_file_destination(dest)?;

        let parent = parent_directory(dest);
        create_dir_all_0755(&parent)?;

        let blob = self.path_for(&entry.blake3);
        let mut source = File::open(&blob).map_err(|source| Error::io(&blob, source))?;
        let mut temp = tempfile::NamedTempFile::new_in(&parent)
            .map_err(|source| Error::io(&parent, source))?;
        let temp_path = temp.path().to_path_buf();
        std::io::copy(&mut source, temp.as_file_mut())
            .map_err(|source| Error::io(&temp_path, source))?;
        temp.as_file_mut()
            .flush()
            .map_err(|source| Error::io(&temp_path, source))?;
        set_mode(&temp_path, 0o644)?;
        temp.as_file()
            .sync_all()
            .map_err(|source| Error::io(&temp_path, source))?;
        temp.persist(dest)
            .map_err(|error| Error::io(dest, error.error))?;

        Ok(())
    }

    /// Creates `<root>/blobs` and `<root>/tmp` when missing.
    fn ensure_directories(&self) -> Result<()> {
        for directory in [self.root.join(BLOBS_DIR), self.root.join(TMP_DIR)] {
            fs::create_dir_all(&directory).map_err(|source| Error::io(&directory, source))?;
        }
        Ok(())
    }

    /// Copies a file that already passed `hash_file`'s checks into a temporary
    /// file and renames it to `blob`.
    ///
    /// The second pass re-hashes the source while copying and rejects the
    /// write when the digest or size changed, so the bytes at `blob` are the
    /// bytes the returned [`Entry`] describes.
    fn write_blob(&self, source: &Path, entry: &Entry, blob: &Path) -> Result<()> {
        let tmp_dir = self.root.join(TMP_DIR);
        fs::create_dir_all(&tmp_dir).map_err(|source| Error::io(&tmp_dir, source))?;
        let mut temp = tempfile::NamedTempFile::new_in(&tmp_dir)
            .map_err(|source| Error::io(&tmp_dir, source))?;
        let temp_path = temp.path().to_path_buf();

        let (sha256, blake3, size) = copy_and_hash(source, &temp_path, temp.as_file_mut())?;
        if (sha256, blake3, size) != (entry.sha256, entry.blake3, entry.size) {
            return Err(Error::Validation(format!(
                "content-addressed store: {} changed while it was stored: expected blake3 {} and {} bytes, found blake3 {blake3} and {size} bytes",
                source.display(),
                entry.blake3,
                entry.size
            )));
        }

        temp.as_file_mut()
            .flush()
            .map_err(|source| Error::io(&temp_path, source))?;
        set_mode(&temp_path, 0o644)?;
        temp.as_file()
            .sync_all()
            .map_err(|source| Error::io(&temp_path, source))?;

        if let Some(parent) = blob.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
        }
        temp.persist(blob)
            .map_err(|error| Error::io(blob, error.error))?;

        Ok(())
    }
}

/// Returns the default content-addressed store root.
///
/// The first non-empty value of `HOARD_STORE`, `$XDG_CACHE_HOME/hoard/store`,
/// and `$HOME/.cache/hoard/store` wins, in that order. When none of those
/// variables is set, the relative path `.cache/hoard/store` is returned.
pub fn store_root_from_env() -> PathBuf {
    if let Some(root) = environment_value("HOARD_STORE") {
        return PathBuf::from(root);
    }
    if let Some(cache) = environment_value("XDG_CACHE_HOME") {
        return PathBuf::from(cache).join("hoard").join("store");
    }
    if let Some(home) = environment_value("HOME") {
        return PathBuf::from(home)
            .join(".cache")
            .join("hoard")
            .join("store");
    }
    PathBuf::from(".cache").join("hoard").join("store")
}

/// Returns the value of an environment variable, treating an empty value as
/// unset.
fn environment_value(name: &str) -> Option<OsString> {
    std::env::var_os(name).filter(|value| !value.is_empty())
}

/// Hashes the file at `path` with SHA-256 and BLAKE3 in one streaming pass.
fn hash_file(path: &Path) -> Result<(Hash32, Hash32, u64)> {
    let mut file = File::open(path).map_err(|source| Error::io(path, source))?;
    let mut sha256 = Sha256::new();
    let mut blake3 = Blake3Hasher::new();
    let mut size = 0u64;
    let mut buffer = vec![0u8; BUFFER_SIZE];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| Error::io(path, source))?;
        if read == 0 {
            break;
        }
        sha256.update(&buffer[..read]);
        blake3.update(&buffer[..read]);
        size = checked_size(size, read, path)?;
    }
    Ok(finish(sha256, blake3, size))
}

/// Copies the file at `source` to `destination`, hashing it in the same pass.
fn copy_and_hash(
    source: &Path,
    destination_path: &Path,
    destination: &mut File,
) -> Result<(Hash32, Hash32, u64)> {
    let mut file = File::open(source).map_err(|error| Error::io(source, error))?;
    let mut sha256 = Sha256::new();
    let mut blake3 = Blake3Hasher::new();
    let mut size = 0u64;
    let mut buffer = vec![0u8; BUFFER_SIZE];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| Error::io(source, error))?;
        if read == 0 {
            break;
        }
        sha256.update(&buffer[..read]);
        blake3.update(&buffer[..read]);
        destination
            .write_all(&buffer[..read])
            .map_err(|source| Error::io(destination_path, source))?;
        size = checked_size(size, read, source)?;
    }
    Ok(finish(sha256, blake3, size))
}

/// Adds the number of bytes just read to the running size, failing on overflow.
fn checked_size(size: u64, read: usize, path: &Path) -> Result<u64> {
    size.checked_add(read as u64).ok_or_else(|| {
        Error::Validation(format!(
            "content-addressed store: file {} is too large to size",
            path.display()
        ))
    })
}

/// Finalises both hashers into digest values.
fn finish(sha256: Sha256, blake3: Blake3Hasher, size: u64) -> (Hash32, Hash32, u64) {
    let mut sha256_bytes = [0u8; 32];
    sha256_bytes.copy_from_slice(&sha256.finalize());
    let blake3 = Hash32::from_bytes(*blake3.finalize().as_bytes());
    (Hash32::from_bytes(sha256_bytes), blake3, size)
}

/// Refuses a destination that is a symlink or a directory.
fn refuse_non_file_destination(dest: &Path) -> Result<()> {
    match fs::symlink_metadata(dest) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(Error::Validation(format!(
            "content-addressed store: refusing to write through symlink {}",
            dest.display()
        ))),
        Ok(metadata) if metadata.is_dir() => Err(Error::Validation(format!(
            "content-addressed store: destination {} is a directory",
            dest.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(dest, error)),
    }
}

/// Returns the directory that should hold a temporary file for `path`, using
/// the current directory for a bare file name.
fn parent_directory(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Creates `path` and its missing ancestors, giving every created directory
/// mode `0755` (explicitly, independent of the umask).
fn create_dir_all_0755(path: &Path) -> Result<()> {
    let mut missing = Vec::new();
    let mut current = Some(path);
    while let Some(directory) = current {
        match fs::symlink_metadata(directory) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(directory.to_path_buf());
                current = directory
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty());
            }
            Err(error) => return Err(Error::io(directory, error)),
        }
    }

    for directory in missing.iter().rev() {
        match fs::create_dir(directory) {
            Ok(()) => set_mode(directory, 0o755)?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(Error::io(directory, error)),
        }
    }

    Ok(())
}

/// Sets the mode of `path` to `mode` on Unix; a no-op elsewhere.
#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|source| Error::io(path, source))
}

/// Sets the mode of `path` to `mode` on Unix; a no-op elsewhere.
#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}
