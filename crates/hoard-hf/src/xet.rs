//! Xet transport built on `hf-xet`.

use std::fs;
use std::path::{Path, PathBuf};

use xet::xet_session::{HeaderMap, HeaderValue, XetFileInfo, XetSessionBuilder, header};

use crate::error::{Error, Result};
use crate::hub::{self, HubClient, RemoteFile};
use crate::repo::{CommitSha, RepoSpec};

/// Download one file over Xet into `<dest>.part`, verify it, and rename it to
/// `dest`.
pub(crate) fn download_file(
    client: &HubClient,
    spec: &RepoSpec,
    rev: &CommitSha,
    file: &RemoteFile,
    dest: &Path,
    expected_sha: &str,
) -> Result<u64> {
    let xet_hash = file
        .xet_hash
        .as_deref()
        .ok_or_else(|| Error::MissingXetHash {
            path: file.path.clone(),
        })?;
    let part = hub::part_path(dest);
    hub::ensure_parent(dest)?;

    // The Hub redirects xet-read-token to the canonical repository path, so
    // resolve the final URL before handing it to the Xet session.
    let token_url = client.canonical_xet_token_url(spec, rev)?;
    let cache = XetCache::new()?;
    let session = XetSessionBuilder::new()
        .with_cache_dir(cache.path())
        .build()
        .map_err(|error| xet_error(file, error))?;

    let mut headers = HeaderMap::new();
    if let Some(token) = client.token() {
        let value = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| xet_error(file, "the access token is not a valid header value"))?;
        headers.insert(header::AUTHORIZATION, value);
    }

    let info = XetFileInfo {
        hash: xet_hash.to_string(),
        file_size: Some(file.size),
        sha256: Some(expected_sha.to_string()),
    };

    let group = session
        .new_file_download_group()
        .map_err(|error| xet_error(file, error))?
        .with_token_refresh_url(token_url.as_str(), headers)
        .build_blocking()
        .map_err(|error| xet_error(file, error))?;

    if let Err(error) = group.download_file_to_path_blocking(info, part.clone()) {
        return Err(xet_error(file, error));
    }
    if let Err(error) = group.finish_blocking() {
        let _ = fs::remove_file(&part);
        return Err(xet_error(file, error));
    }

    hub::finish_download(&part, dest, file.size, expected_sha)
}

fn xet_error(file: &RemoteFile, error: impl std::fmt::Display) -> Error {
    Error::Xet {
        path: file.path.clone(),
        message: error.to_string(),
    }
}

/// Where the Xet session may write its shard cache and staging files.
enum XetCache {
    /// A directory named by `HOARD_XET_CACHE`.
    Env(PathBuf),
    /// A per-download temporary directory, removed when the download ends.
    Temp(tempfile::TempDir),
}

impl XetCache {
    fn new() -> Result<Self> {
        if let Ok(value) = std::env::var("HOARD_XET_CACHE") {
            let value = value.trim();
            if !value.is_empty() {
                let path = PathBuf::from(value);
                fs::create_dir_all(&path).map_err(|error| Error::io(&path, error))?;
                return Ok(Self::Env(path));
            }
        }
        let temp = tempfile::Builder::new()
            .prefix("hoard-xet-cache-")
            .tempdir()
            .map_err(|error| Error::io(std::env::temp_dir(), error))?;
        Ok(Self::Temp(temp))
    }

    fn path(&self) -> &Path {
        match self {
            Self::Env(path) => path,
            Self::Temp(temp) => temp.path(),
        }
    }
}
