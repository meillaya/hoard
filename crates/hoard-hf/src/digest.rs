//! SHA-256 helpers used to verify downloads.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

const SHA256_BYTES: usize = 32;
const SHA256_HEX: usize = 64;

/// Decode a 64 character lowercase hexadecimal SHA-256 digest.
pub(crate) fn decode_sha256(value: &str) -> Result<[u8; SHA256_BYTES]> {
    let valid = value.len() == SHA256_HEX
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !valid {
        return Err(Error::InvalidDigest(value.to_string()));
    }

    let mut out = [0u8; SHA256_BYTES];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        out[index] = (hex_value(pair[0]) << 4) | hex_value(pair[1]);
    }
    Ok(out)
}

/// Encode bytes as lowercase hexadecimal.
pub(crate) fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => unreachable!("digest was validated as lowercase hex"),
    }
}

/// A streaming SHA-256 hasher.
#[derive(Debug)]
pub(crate) struct FileHasher(Sha256);

impl FileHasher {
    /// Start a hasher.
    pub(crate) fn new() -> Self {
        Self(Sha256::new())
    }

    /// Feed bytes into the hasher.
    pub(crate) fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    /// Finish hashing and return the lowercase hexadecimal digest.
    pub(crate) fn finish_hex(self) -> String {
        encode_hex(&self.0.finalize())
    }
}

/// Hash a file's full contents as lowercase hexadecimal SHA-256.
pub(crate) fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|e| Error::io(path, e))?;
    let mut hasher = FileHasher::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|e| Error::io(path, e))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finish_hex())
}

/// Extend a hasher with the current contents of `path`, from `offset`.
pub(crate) fn hash_existing(path: &Path, offset: u64, hasher: &mut FileHasher) -> Result<()> {
    let mut file = File::open(path).map_err(|e| Error::io(path, e))?;
    if offset > 0 {
        file.seek(SeekFrom::Start(0))
            .map_err(|e| Error::io(path, e))?;
    }
    let mut buffer = vec![0u8; 64 * 1024];
    let mut remaining = offset;
    while remaining > 0 {
        let want = buffer.len().min(remaining as usize);
        let read = file
            .read(&mut buffer[..want])
            .map_err(|e| Error::io(path, e))?;
        if read == 0 {
            return Err(Error::io(
                path,
                std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "partial file ended before the requested resume offset",
                ),
            ));
        }
        hasher.update(&buffer[..read]);
        remaining -= read as u64;
    }
    Ok(())
}

/// Verify size and SHA-256 of a completed download.
///
/// Returns the verified size on success. On mismatch the caller decides whether
/// to keep or remove the file.
pub(crate) fn verify_file(path: &Path, expected_size: u64, expected_sha: &str) -> Result<u64> {
    let metadata = std::fs::metadata(path).map_err(|e| Error::io(path, e))?;
    let actual_size = metadata.len();
    if actual_size != expected_size {
        return Err(Error::SizeMismatch {
            path: path.to_path_buf(),
            expected: expected_size,
            actual: actual_size,
        });
    }
    let actual_sha = sha256_file(path)?;
    if actual_sha != expected_sha {
        return Err(Error::DigestMismatch {
            path: path.to_path_buf(),
            expected: expected_sha.to_string(),
            actual: actual_sha,
        });
    }
    Ok(actual_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY_SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn hex_round_trip() {
        let bytes = decode_sha256(EMPTY_SHA).unwrap();
        assert_eq!(encode_hex(&bytes), EMPTY_SHA);
    }

    #[test]
    fn decode_rejects_bad_digests() {
        for bad in [
            "",
            "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b85",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b85g",
        ] {
            assert!(decode_sha256(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn hashes_known_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.bin");
        std::fs::write(&path, b"").unwrap();
        assert_eq!(sha256_file(&path).unwrap(), EMPTY_SHA);
        assert_eq!(verify_file(&path, 0, EMPTY_SHA).unwrap(), 0);

        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(matches!(
            verify_file(&path, 3, EMPTY_SHA),
            Err(Error::DigestMismatch { .. })
        ));
        assert!(matches!(
            verify_file(&path, 4, EMPTY_SHA),
            Err(Error::SizeMismatch { .. })
        ));
    }

    #[test]
    fn hashes_an_existing_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prefix.bin");
        std::fs::write(&path, b"abcdef").unwrap();

        let mut hasher = FileHasher::new();
        hash_existing(&path, 3, &mut hasher).unwrap();
        hasher.update(b"def");
        assert_eq!(
            hasher.finish_hex(),
            "bef57ec7f53a6d40beb640a780a639c83bc29ac8a9816f1fc6c5c6dcd93c4721"
        );
    }
}
