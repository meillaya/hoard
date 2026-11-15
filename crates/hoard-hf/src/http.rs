//! Plain HTTP transport with Range resume and digest verification.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use reqwest::StatusCode;
use reqwest::header::RANGE;

use crate::digest::{self, FileHasher};
use crate::error::{Error, Result};
use crate::hub::{self, HubClient, RemoteFile};
use crate::redirect;
use crate::repo::{CommitSha, RepoSpec};

const CHUNK_SIZE: usize = 64 * 1024;

/// Download one file over plain HTTP into `<dest>.part`, verify it, and rename
/// it to `dest`.
pub(crate) fn download_file(
    client: &HubClient,
    spec: &RepoSpec,
    rev: &CommitSha,
    file: &RemoteFile,
    dest: &Path,
    expected_sha: &str,
) -> Result<u64> {
    let url = client.http_file_url(spec, rev, &file.path)?;
    let context = format!("GET {}", redirect::display_url(&url));
    let part = hub::part_path(dest);
    hub::ensure_parent(dest)?;

    let expected_size = file.size;
    let mut resume_from = match fs::metadata(&part) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(Error::io(&part, error)),
    };
    if resume_from > expected_size {
        fs::remove_file(&part).map_err(|error| Error::io(&part, error))?;
        resume_from = 0;
    }
    if resume_from > 0 && resume_from == expected_size {
        // The previous attempt finished the body but did not get to rename it.
        return hub::finish_download(&part, dest, expected_size, expected_sha);
    }

    let mut response = client.send_with_retry(&context, || {
        let mut request = client.request(&url)?;
        if resume_from > 0 {
            let value = reqwest::header::HeaderValue::from_str(&format!("bytes={resume_from}-"))
                .expect("a byte range header is always a valid header value");
            request.headers_mut().insert(RANGE, value);
        }
        Ok(request)
    })?;

    let status = response.status();
    let declared_length = response.content_length();
    let mut hasher = FileHasher::new();
    let mut output: File;
    let expected_this_response: u64;

    if status == StatusCode::PARTIAL_CONTENT {
        let header_value = response
            .headers()
            .get("content-range")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let Some((start, Some(end), total)) = parse_content_range(&header_value) else {
            return Err(Error::InvalidContentRange {
                context,
                value: header_value,
            });
        };
        if start != resume_from || end.checked_add(1) != Some(expected_size) {
            return Err(Error::InvalidContentRange {
                context,
                value: header_value,
            });
        }
        if let Some(total) = total
            && total != expected_size
        {
            return Err(Error::SizeMismatch {
                path: part.clone(),
                expected: expected_size,
                actual: total,
            });
        }
        expected_this_response = expected_size - resume_from;
        if let Some(declared) = declared_length
            && declared != expected_this_response
        {
            return Err(Error::TruncatedBody {
                context,
                expected: expected_this_response,
                actual: declared,
            });
        }
        digest::hash_existing(&part, resume_from, &mut hasher)?;
        output = OpenOptions::new()
            .append(true)
            .open(&part)
            .map_err(|error| Error::io(&part, error))?;
    } else if status == StatusCode::OK {
        // A 200 after a range request means the server ignored the range, so
        // start over from a clean part file.
        if let Some(declared) = declared_length
            && declared != expected_size
        {
            return Err(Error::SizeMismatch {
                path: part.clone(),
                expected: expected_size,
                actual: declared,
            });
        }
        expected_this_response = expected_size;
        output = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&part)
            .map_err(|error| Error::io(&part, error))?;
    } else {
        return Err(Error::Status {
            status: status.as_u16(),
            context,
        });
    }

    let mut received: u64 = 0;
    let mut buffer = vec![0u8; CHUNK_SIZE];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|error| Error::io(&part, error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        output
            .write_all(&buffer[..read])
            .map_err(|error| Error::io(&part, error))?;
        received += read as u64;
    }
    output.flush().map_err(|error| Error::io(&part, error))?;
    drop(output);

    if received != expected_this_response {
        return Err(Error::TruncatedBody {
            context,
            expected: expected_this_response,
            actual: received,
        });
    }

    let actual_sha = hasher.finish_hex();
    if actual_sha != expected_sha {
        let _ = fs::remove_file(&part);
        return Err(Error::DigestMismatch {
            path: part,
            expected: expected_sha.to_string(),
            actual: actual_sha,
        });
    }

    let written = fs::metadata(&part)
        .map_err(|error| Error::io(&part, error))?
        .len();
    if written != expected_size {
        return Err(Error::SizeMismatch {
            path: part,
            expected: expected_size,
            actual: written,
        });
    }

    fs::rename(&part, dest).map_err(|error| Error::io(dest, error))?;
    Ok(expected_size)
}

/// Parse a `Content-Range` value of the form `bytes start-end/total`.
///
/// `total` is `None` for `*`. Malformed values return `None`.
pub(crate) fn parse_content_range(value: &str) -> Option<(u64, Option<u64>, Option<u64>)> {
    let rest = value.strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start = start.parse().ok()?;
    let end = if end.is_empty() {
        None
    } else {
        Some(end.parse().ok()?)
    };
    let total = if total == "*" {
        None
    } else {
        Some(total.parse().ok()?)
    };
    Some((start, end, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_parsing() {
        assert_eq!(
            parse_content_range("bytes 0-99/100"),
            Some((0, Some(99), Some(100)))
        );
        assert_eq!(
            parse_content_range("bytes 100-199/200"),
            Some((100, Some(199), Some(200)))
        );
        assert_eq!(
            parse_content_range("bytes 100-199/*"),
            Some((100, Some(199), None))
        );
        assert_eq!(
            parse_content_range("bytes 100-199/200 "),
            None,
            "trailing data is rejected"
        );
        assert_eq!(parse_content_range("bytes */200"), None);
        assert_eq!(parse_content_range(""), None);
        assert_eq!(parse_content_range("items 0-1/2"), None);
    }
}
