//! Streaming file hashing and the digest types used by manifests.
//!
//! SHA-256 digests come from the source (the Hugging Face LFS oid) and are
//! recorded for provenance; BLAKE3 digests are computed by hoard and identify
//! content in the content-addressed store.

use std::borrow::Cow;
use std::fmt;
use std::io::Read;
use std::path::Path;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{Error, Result};

/// Number of bytes read from a file per hashing step.
const BUFFER_SIZE: usize = 1024 * 1024;

/// Number of bytes in a SHA-256 or BLAKE3 digest.
const DIGEST_LEN: usize = 32;

/// A 32-byte digest, rendered and serialised as 64 lowercase hexadecimal
/// characters.
///
/// Parse one from hexadecimal with [`FromStr`]; hash files with
/// [`sha256_file`] and [`blake3_file`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash32([u8; DIGEST_LEN]);

impl Hash32 {
    /// Wraps raw digest bytes.
    pub const fn from_bytes(bytes: [u8; DIGEST_LEN]) -> Self {
        Self(bytes)
    }

    /// Returns the raw digest bytes.
    pub const fn as_bytes(&self) -> &[u8; DIGEST_LEN] {
        &self.0
    }

    /// Returns the digest as 64 lowercase hexadecimal characters.
    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for Hash32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

impl FromStr for Hash32 {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let bytes = parse_hex32(value)?;
        Ok(Self(bytes))
    }
}

impl Serialize for Hash32 {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Hash32 {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Hash32 {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "Hash32".into()
    }

    fn schema_id() -> Cow<'static, str> {
        concat!(module_path!(), "::Hash32").into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "Digest: exactly 64 lowercase hexadecimal characters.",
            "pattern": "^[0-9a-f]{64}$"
        })
    }
}

/// A BLAKE3 digest of a manifest's canonical JSON bytes.
///
/// Displays and serialises as `blake3:<64 lowercase hex>`; parsing also accepts
/// the bare 64-character hexadecimal form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Blake3Digest(Hash32);

impl Blake3Digest {
    /// Wraps a 32-byte BLAKE3 digest.
    pub const fn from_hash32(digest: Hash32) -> Self {
        Self(digest)
    }

    /// Returns the wrapped digest.
    pub const fn as_hash32(&self) -> &Hash32 {
        &self.0
    }

    /// Returns the raw digest bytes.
    pub const fn as_bytes(&self) -> &[u8; DIGEST_LEN] {
        self.0.as_bytes()
    }

    /// Returns the digest as 64 lowercase hexadecimal characters, without the
    /// `blake3:` prefix.
    pub fn to_hex(self) -> String {
        self.0.to_hex()
    }
}

impl fmt::Display for Blake3Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "blake3:{}", self.0)
    }
}

impl FromStr for Blake3Digest {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let hex_part = value.strip_prefix("blake3:").unwrap_or(value);
        Ok(Self(Hash32::from_str(hex_part)?))
    }
}

impl Serialize for Blake3Digest {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Blake3Digest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Blake3Digest {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "Blake3Digest".into()
    }

    fn schema_id() -> Cow<'static, str> {
        concat!(module_path!(), "::Blake3Digest").into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "BLAKE3 digest, written as `blake3:` followed by 64 lowercase hexadecimal characters.",
            "pattern": "^blake3:[0-9a-f]{64}$"
        })
    }
}

/// Returns the SHA-256 digest of the file at `path`, read in 1 MiB chunks.
pub fn sha256_file(path: &Path) -> Result<Hash32> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    read_in_chunks(path, |chunk| {
        hasher.update(chunk);
    })?;

    let mut bytes = [0u8; DIGEST_LEN];
    bytes.copy_from_slice(&hasher.finalize());
    Ok(Hash32(bytes))
}

/// Returns the BLAKE3 digest of the file at `path`, read in 1 MiB chunks.
pub fn blake3_file(path: &Path) -> Result<Hash32> {
    let mut hasher = blake3::Hasher::new();
    read_in_chunks(path, |chunk| {
        hasher.update(chunk);
    })?;
    Ok(Hash32(*hasher.finalize().as_bytes()))
}

/// Returns the BLAKE3 digest of an in-memory byte slice.
pub fn blake3_bytes(bytes: &[u8]) -> Hash32 {
    Hash32(*blake3::hash(bytes).as_bytes())
}

/// Feeds the contents of `path` to `update` in 1 MiB chunks.
fn read_in_chunks(path: &Path, mut update: impl FnMut(&[u8])) -> Result<()> {
    let mut file = std::fs::File::open(path).map_err(|source| Error::io(path, source))?;
    let mut buffer = vec![0u8; BUFFER_SIZE];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| Error::io(path, source))?;
        if read == 0 {
            return Ok(());
        }
        update(&buffer[..read]);
    }
}

/// Parses exactly 64 lowercase hexadecimal characters into 32 bytes.
fn parse_hex32(value: &str) -> Result<[u8; DIGEST_LEN]> {
    if value.len() != 2 * DIGEST_LEN {
        return Err(Error::InvalidDigest {
            value: value.to_owned(),
            message: format!(
                "expected {} hexadecimal characters, found {}",
                2 * DIGEST_LEN,
                value.len()
            ),
        });
    }
    if !value.bytes().all(is_lowercase_hex) {
        return Err(Error::InvalidDigest {
            value: value.to_owned(),
            message: "expected only lowercase hexadecimal characters [0-9a-f]".to_owned(),
        });
    }

    let mut bytes = [0u8; DIGEST_LEN];
    hex::decode_to_slice(value, &mut bytes).map_err(|error| Error::InvalidDigest {
        value: value.to_owned(),
        message: error.to_string(),
    })?;
    Ok(bytes)
}

/// Returns whether `byte` is a lowercase hexadecimal digit.
fn is_lowercase_hex(byte: u8) -> bool {
    matches!(byte, b'0'..=b'9' | b'a'..=b'f')
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    #[test]
    fn hash32_parses_and_displays_lowercase_hex() {
        let hash: Hash32 = HEX.parse().unwrap();
        assert_eq!(hash.to_string(), HEX);
        assert_eq!(hash.to_hex(), HEX);
        assert_eq!(
            hash,
            Hash32::from_bytes([
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23, 24, 25, 26, 27, 28, 29, 30, 31,
            ])
        );
    }

    #[test]
    fn hash32_rejects_uppercase() {
        let uppercase = HEX.to_uppercase();
        let error = uppercase.parse::<Hash32>().unwrap_err();
        assert!(error.to_string().contains("lowercase"), "{error}");
    }

    #[test]
    fn hash32_rejects_wrong_length_and_non_hex() {
        for value in ["", "00", &HEX[..62], &format!("{HEX}00")] {
            assert!(
                value.parse::<Hash32>().is_err(),
                "{value:?} should have been rejected"
            );
        }
        let mut non_hex = HEX.to_owned();
        non_hex.replace_range(..1, "g");
        assert!(non_hex.parse::<Hash32>().is_err());

        let mut with_prefix = HEX.to_owned();
        with_prefix.insert_str(0, "blake3:");
        assert!(with_prefix.parse::<Hash32>().is_err());
    }

    #[test]
    fn hash32_serde_uses_hex_string() {
        let hash: Hash32 = HEX.parse().unwrap();
        let serialised = serde_json::to_string(&hash).unwrap();
        assert_eq!(serialised, format!("\"{HEX}\""));
        let parsed: Hash32 = serde_json::from_str(&serialised).unwrap();
        assert_eq!(parsed, hash);

        assert!(serde_json::from_str::<Hash32>("\"NOT-A-HASH\"").is_err());
        assert!(serde_json::from_str::<Hash32>("null").is_err());
    }

    #[test]
    fn blake3_digest_displays_with_prefix() {
        let hash: Hash32 = HEX.parse().unwrap();
        let digest = Blake3Digest::from_hash32(hash);
        assert_eq!(digest.to_string(), format!("blake3:{HEX}"));
        assert_eq!(digest.to_hex(), HEX);
        assert_eq!(digest.as_bytes(), hash.as_bytes());

        let parsed: Blake3Digest = digest.to_string().parse().unwrap();
        assert_eq!(parsed, digest);
        let bare: Blake3Digest = HEX.parse().unwrap();
        assert_eq!(bare, digest);
    }

    #[test]
    fn blake3_digest_serde_uses_prefixed_string() {
        let digest = Blake3Digest::from_hash32(HEX.parse().unwrap());
        let serialised = serde_json::to_value(digest).unwrap();
        assert_eq!(serialised, serde_json::json!(format!("blake3:{HEX}")));
        let parsed: Blake3Digest = serde_json::from_value(serialised).unwrap();
        assert_eq!(parsed, digest);

        let bare: Blake3Digest = serde_json::from_value(serde_json::json!(HEX)).unwrap();
        assert_eq!(bare, digest);
        assert!(serde_json::from_value::<Blake3Digest>(serde_json::json!("blake3:zz")).is_err());
    }
}
