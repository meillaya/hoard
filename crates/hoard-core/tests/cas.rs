//! Content-addressed store tests.
//!
//! Every test runs in a temporary directory; there is no network and no shared
//! state between tests.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use sha2::{Digest, Sha256};

use hoard_core::cas::{Store, store_root_from_env};
use hoard_core::{Entry, Error, Hash32};

/// SHA-256 of `b"hello"`.
const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
/// BLAKE3 of `b"hello"`.
const HELLO_BLAKE3: &str = "ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f";

/// Parses a known digest.
fn hash(value: &str) -> Hash32 {
    value.parse().expect("test digest should be valid")
}

/// Creates a store in a fresh temporary directory.
fn temp_store() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let store = Store::open(directory.path().join("store")).expect("store opens");
    (directory, store)
}

/// Writes `contents` to `directory/name` and returns the path.
fn write_file(directory: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = directory.join(name);
    let mut file = fs::File::create(&path).expect("source file");
    file.write_all(contents).expect("write source");
    file.sync_all().expect("sync source");
    path
}

/// Computes SHA-256 of `contents` through the reference crate.
fn sha256(contents: &[u8]) -> Hash32 {
    let mut digest = Sha256::new();
    digest.update(contents);
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&digest.finalize());
    Hash32::from_bytes(bytes)
}

#[test]
fn open_creates_the_store_layout() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("nested/store");
    let store = Store::open(root.clone()).unwrap();

    assert_eq!(store.root(), root.as_path());
    assert!(root.join("blobs").is_dir(), "blobs directory");
    assert!(root.join("tmp").is_dir(), "tmp directory");
}

#[test]
fn put_file_hashes_checks_and_stores_a_file() {
    let (directory, store) = temp_store();
    let source = write_file(directory.path(), "hello.txt", b"hello");
    let expected = hash(HELLO_SHA256);

    let entry = store.put_file(&source, expected).unwrap();

    assert_eq!(entry.sha256, expected);
    assert_eq!(entry.blake3, hash(HELLO_BLAKE3));
    assert_eq!(entry.size, 5);

    let blob = store.path_for(&entry.blake3);
    assert_eq!(
        blob,
        store.root().join("blobs").join("ea").join(HELLO_BLAKE3),
        "blob must live at blobs/<first two hex>/<64 hex>"
    );
    assert_eq!(fs::read(&blob).unwrap(), b"hello");
    assert!(store.contains(&entry.blake3));
    store.verify(&entry).unwrap();
    assert_eq!(
        fs::read_dir(store.root().join("tmp")).unwrap().count(),
        0,
        "no temporary files may be left behind"
    );
}

#[test]
fn put_file_rejects_a_wrong_sha256_and_leaves_no_blob() {
    let (directory, store) = temp_store();
    let source = write_file(directory.path(), "hello.txt", b"hello");

    let error = store.put_file(&source, hash(HELLO_BLAKE3)).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("sha256 mismatch"), "{message}");
    assert!(message.contains(HELLO_BLAKE3), "expected value: {message}");
    assert!(message.contains(HELLO_SHA256), "computed value: {message}");
    assert!(message.contains("hello.txt"), "offending path: {message}");

    assert!(
        !store.contains(&hash(HELLO_BLAKE3)),
        "no blob may be stored"
    );
    assert_eq!(
        fs::read_dir(store.root().join("blobs")).unwrap().count(),
        0,
        "blobs directory must stay empty"
    );
    assert_eq!(
        fs::read_dir(store.root().join("tmp")).unwrap().count(),
        0,
        "no partial file may be left behind"
    );
}

#[test]
fn put_file_reports_a_missing_source() {
    let (directory, store) = temp_store();
    let missing = directory.path().join("nope.bin");

    let error = store.put_file(&missing, hash(HELLO_SHA256)).unwrap_err();
    assert!(matches!(error, Error::Io { .. }), "{error}");
    assert!(error.to_string().contains("nope.bin"), "{error}");
}

#[test]
fn put_file_is_idempotent_for_the_same_content() {
    let (directory, store) = temp_store();
    let source = write_file(directory.path(), "hello.txt", b"hello");
    let expected = hash(HELLO_SHA256);

    let first = store.put_file(&source, expected).unwrap();
    let second = store.put_file(&source, expected).unwrap();

    assert_eq!(first, second);
    store.verify(&second).unwrap();
    assert_eq!(fs::read_dir(store.root().join("tmp")).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn put_file_keeps_an_existing_verified_blob() {
    use std::os::unix::fs::MetadataExt;

    let (directory, store) = temp_store();
    let source = write_file(directory.path(), "hello.txt", b"hello");
    let expected = hash(HELLO_SHA256);

    let entry = store.put_file(&source, expected).unwrap();
    let blob = store.path_for(&entry.blake3);
    let inode_before = fs::metadata(&blob).unwrap().ino();

    store.put_file(&source, expected).unwrap();

    assert_eq!(
        fs::metadata(&blob).unwrap().ino(),
        inode_before,
        "a verified blob must be reused, not replaced"
    );
}

#[test]
fn put_file_repairs_a_corrupt_blob() {
    let (directory, store) = temp_store();
    let source = write_file(directory.path(), "hello.txt", b"hello");
    let expected = hash(HELLO_SHA256);
    let entry = store.put_file(&source, expected).unwrap();
    let blob = store.path_for(&entry.blake3);

    fs::write(&blob, b"HELLO").unwrap();
    store.verify(&entry).unwrap_err();

    let repaired = store.put_file(&source, expected).unwrap();
    assert_eq!(repaired, entry);
    store.verify(&repaired).unwrap();
    assert_eq!(fs::read(&blob).unwrap(), b"hello");
}

#[test]
fn verify_rejects_a_corrupt_blob() {
    let (directory, store) = temp_store();
    let source = write_file(directory.path(), "hello.txt", b"hello");
    let entry = store.put_file(&source, hash(HELLO_SHA256)).unwrap();
    let blob = store.path_for(&entry.blake3);

    // Same size, different bytes: only the digests can catch this.
    fs::write(&blob, b"HELLO").unwrap();

    let error = store.verify(&entry).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("corrupt blob"), "{message}");
    assert!(message.contains("expected blake3"), "{message}");
    assert!(message.contains(HELLO_BLAKE3), "{message}");
    assert!(message.contains(blob.to_str().unwrap()), "{message}");
}

#[test]
fn verify_rejects_a_truncated_blob() {
    let (directory, store) = temp_store();
    let source = write_file(directory.path(), "hello.txt", b"hello");
    let entry = store.put_file(&source, hash(HELLO_SHA256)).unwrap();
    let blob = store.path_for(&entry.blake3);

    fs::write(&blob, b"hel").unwrap();

    let error = store.verify(&entry).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("corrupt blob"), "{message}");
    assert!(
        message.contains("expected 5 bytes, found 3"),
        "size mismatch must be reported: {message}"
    );
}

#[test]
fn verify_rejects_a_missing_blob() {
    let (directory, store) = temp_store();
    let source = write_file(directory.path(), "hello.txt", b"hello");
    let entry = store.put_file(&source, hash(HELLO_SHA256)).unwrap();
    fs::remove_file(store.path_for(&entry.blake3)).unwrap();

    let error = store.verify(&entry).unwrap_err();
    assert!(matches!(error, Error::Io { .. }), "{error}");
    assert!(error.to_string().contains(HELLO_BLAKE3), "{error}");
}

#[test]
fn contains_reports_presence_without_verifying() {
    let (_directory, store) = temp_store();
    let digest = hash(HELLO_BLAKE3);
    assert!(!store.contains(&digest));

    // A poisoned blob has the right name but the wrong bytes.
    let blob = store.path_for(&digest);
    fs::create_dir_all(blob.parent().unwrap()).unwrap();
    fs::write(&blob, b"poison").unwrap();

    assert!(store.contains(&digest), "contains only checks the file");
}

#[test]
fn materialize_copies_the_blob_with_mode_0644() {
    let (directory, store) = temp_store();
    let source = write_file(directory.path(), "hello.txt", b"hello");
    let entry = store.put_file(&source, hash(HELLO_SHA256)).unwrap();
    let dest = directory.path().join("out/nested/hello.txt");

    store.materialize(&entry, &dest).unwrap();

    assert_eq!(fs::read(&dest).unwrap(), b"hello");
    assert!(store.contains(&entry.blake3), "materialize is a copy");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let file_mode = fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o644, "materialized file mode");
        let nested_mode = fs::metadata(dest.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(nested_mode, 0o755, "created parent mode");
        let out_mode = fs::metadata(directory.path().join("out"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(out_mode, 0o755, "created grandparent mode");
    }
}

#[test]
fn materialize_replaces_an_existing_file() {
    let (directory, store) = temp_store();
    let source = write_file(directory.path(), "source.bin", b"hello");
    let entry = store.put_file(&source, hash(HELLO_SHA256)).unwrap();
    let out = directory.path().join("out");
    fs::create_dir(&out).unwrap();
    let dest = out.join("hello.txt");
    fs::write(&dest, b"old contents").unwrap();

    store.materialize(&entry, &dest).unwrap();

    assert_eq!(fs::read(&dest).unwrap(), b"hello");
    assert_eq!(
        fs::read_dir(&out).unwrap().count(),
        1,
        "no temporary file may be left next to the destination"
    );
}

#[test]
fn materialize_verifies_the_blob_before_writing() {
    let (directory, store) = temp_store();
    let entry = Entry {
        blake3: hash(HELLO_BLAKE3),
        sha256: hash(HELLO_SHA256),
        size: 5,
    };
    let dest = directory.path().join("never.txt");

    let error = store.materialize(&entry, &dest).unwrap_err();
    assert!(matches!(error, Error::Io { .. }), "{error}");
    assert!(!dest.exists(), "no destination may be written");
}

#[test]
fn materialize_rejects_a_corrupt_blob() {
    let (directory, store) = temp_store();
    let source = write_file(directory.path(), "source.bin", b"hello");
    let entry = store.put_file(&source, hash(HELLO_SHA256)).unwrap();
    fs::write(store.path_for(&entry.blake3), b"HELLO").unwrap();
    let dest = directory.path().join("out/hello.txt");

    let error = store.materialize(&entry, &dest).unwrap_err();
    assert!(error.to_string().contains("corrupt blob"), "{error}");
    assert!(!dest.exists(), "no destination may be written");
}

#[cfg(unix)]
#[test]
fn materialize_refuses_a_symlink_destination() {
    use std::os::unix::fs::symlink;

    let (directory, store) = temp_store();
    let source = write_file(directory.path(), "hello.txt", b"hello");
    let entry = store.put_file(&source, hash(HELLO_SHA256)).unwrap();
    let target = write_file(directory.path(), "target.txt", b"untouched");
    let dest = directory.path().join("link.txt");
    symlink(&target, &dest).unwrap();

    let error = store.materialize(&entry, &dest).unwrap_err();
    assert!(error.to_string().contains("symlink"), "{error}");
    assert_eq!(fs::read(&target).unwrap(), b"untouched");
    assert!(
        fs::symlink_metadata(&dest)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn materialize_refuses_a_directory_destination() {
    let (directory, store) = temp_store();
    let source = write_file(directory.path(), "hello.txt", b"hello");
    let entry = store.put_file(&source, hash(HELLO_SHA256)).unwrap();
    let dest = directory.path().join("a-directory");
    fs::create_dir(&dest).unwrap();

    let error = store.materialize(&entry, &dest).unwrap_err();
    assert!(error.to_string().contains("directory"), "{error}");
    assert!(dest.is_dir());
}

#[test]
fn concurrent_puts_of_the_same_file_produce_one_blob() {
    let (directory, store) = temp_store();
    let contents: Vec<u8> = (0..256 * 1024).map(|index| (index % 251) as u8).collect();
    let source = write_file(directory.path(), "shared.bin", &contents);
    let expected = sha256(&contents);
    let store = Arc::new(store);

    let mut handles = Vec::new();
    for _ in 0..4 {
        let store = Arc::clone(&store);
        let source = source.clone();
        handles.push(thread::spawn(move || {
            store.put_file(&source, expected).unwrap()
        }));
    }
    let entries: Vec<Entry> = handles
        .into_iter()
        .map(|handle| handle.join().expect("put thread"))
        .collect();

    for entry in &entries {
        assert_eq!(entry, &entries[0]);
        store.verify(entry).unwrap();
    }

    let mut blobs = Vec::new();
    for prefix in fs::read_dir(store.root().join("blobs")).unwrap() {
        for blob in fs::read_dir(prefix.unwrap().path()).unwrap() {
            blobs.push(blob.unwrap().path());
        }
    }
    assert_eq!(blobs.len(), 1, "one digest means one blob");
    assert_eq!(fs::read(&blobs[0]).unwrap(), contents);
    assert_eq!(
        fs::read_dir(store.root().join("tmp")).unwrap().count(),
        0,
        "concurrent puts must clean up their temporary files"
    );
}

#[test]
fn path_for_stays_inside_the_blob_directory() {
    let (directory, store) = temp_store();
    let blob_root = directory.path().join("store").join("blobs");

    for seed in [0u8, 1, 0x7f, 0xff] {
        let digest = Hash32::from_bytes([seed; 32]);
        let path = store.path_for(&digest);
        assert!(
            path.starts_with(&blob_root),
            "{} must stay under {}",
            path.display(),
            blob_root.display()
        );

        let relative = path.strip_prefix(&blob_root).unwrap();
        let components: Vec<String> = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect();
        assert_eq!(components.len(), 2, "{path:?}");
        assert_eq!(components[0], digest.to_hex()[..2]);
        assert_eq!(components[1], digest.to_hex());
        assert!(
            components
                .iter()
                .all(|component| component != "." && component != ".."),
            "{path:?}"
        );
        assert!(
            components[1]
                .chars()
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase()),
            "blob name must be lowercase hex: {path:?}"
        );
    }
}

/// Probe test run in a child process with a controlled environment.
#[test]
#[ignore = "spawned by the store_root_from_env tests with a controlled environment"]
fn store_root_env_probe() {
    let Some(expected) = std::env::var_os("HOARD_TEST_EXPECTED_STORE_ROOT") else {
        return;
    };
    assert_eq!(store_root_from_env(), PathBuf::from(expected));
}

/// Runs [`store_root_env_probe`] in a child process with the given environment.
fn probe_store_root(expected: &str, environment: &[(&str, &str)]) {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "store_root_env_probe",
        "--ignored",
        "--nocapture",
    ]);
    command.env_remove("HOARD_STORE");
    command.env_remove("XDG_CACHE_HOME");
    command.env_remove("HOME");
    command.env("HOARD_TEST_EXPECTED_STORE_ROOT", expected);
    for (variable, value) in environment {
        command.env(variable, value);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "probe for {expected:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn store_root_from_env_prefers_hoard_store() {
    probe_store_root(
        "/var/lib/hoard",
        &[
            ("HOARD_STORE", "/var/lib/hoard"),
            ("XDG_CACHE_HOME", "/xdg"),
            ("HOME", "/home/user"),
        ],
    );
}

#[test]
fn store_root_from_env_uses_the_xdg_cache_directory() {
    probe_store_root(
        "/xdg/hoard/store",
        &[("XDG_CACHE_HOME", "/xdg"), ("HOME", "/home/user")],
    );
}

#[test]
fn store_root_from_env_uses_the_home_cache_as_a_last_resort() {
    probe_store_root("/home/user/.cache/hoard/store", &[("HOME", "/home/user")]);
}

#[test]
fn store_root_from_env_ignores_empty_values() {
    probe_store_root(
        "/xdg/hoard/store",
        &[
            ("HOARD_STORE", ""),
            ("XDG_CACHE_HOME", "/xdg"),
            ("HOME", "/home/user"),
        ],
    );
}

#[test]
fn store_root_from_env_falls_back_to_a_relative_path() {
    probe_store_root(".cache/hoard/store", &[]);
}
