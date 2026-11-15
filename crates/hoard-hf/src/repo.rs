//! Repository references and commit identifiers.

use std::fmt;
use std::str::FromStr;

use crate::error::{Error, Result};

/// The kind of Hugging Face repository a path belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepoKind {
    /// A model repository.
    Model,
    /// A dataset repository.
    Dataset,
    /// A space repository.
    Space,
}

impl RepoKind {
    /// The singular segment used in `/api/{type}s/...` routes.
    pub(crate) fn api_segment(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Dataset => "dataset",
            Self::Space => "space",
        }
    }

    /// The route prefix used by file resolve URLs.
    ///
    /// Model files resolve from the repository root (`/{repo}/resolve/...`).
    /// Datasets and spaces are nested under `datasets/` and `spaces/`.
    pub(crate) fn file_prefix(self) -> &'static str {
        match self {
            Self::Model => "",
            Self::Dataset => "datasets",
            Self::Space => "spaces",
        }
    }
}

/// A repository reference: a kind plus an `owner/name` identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSpec {
    /// Whether this is a model, dataset, or space.
    pub kind: RepoKind,
    /// The repository name, of the form `owner/name`.
    pub repo: String,
}

impl RepoSpec {
    /// Split the repository name into its owner and name halves.
    pub(crate) fn split(&self) -> Result<(&str, &str)> {
        let mut parts = self.repo.split('/');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(owner), Some(name), None) if !owner.is_empty() && !name.is_empty() => {
                Ok((owner, name))
            }
            _ => Err(Error::InvalidRepo(self.repo.clone())),
        }
    }
}

/// A resolved, immutable 40 character commit hash.
///
/// Parsing accepts only lowercase hexadecimal, matching what the Hub returns.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommitSha(String);

impl CommitSha {
    /// Borrow the commit hash as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommitSha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for CommitSha {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let valid = s.len() == 40
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if valid {
            Ok(Self(s.to_string()))
        } else {
            Err(Error::InvalidCommit(s.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "499bc8821c6b12b4e53c5bffcb21ec206f212d81";

    #[test]
    fn commit_sha_round_trips() {
        let sha: CommitSha = SHA.parse().unwrap();
        assert_eq!(sha.as_str(), SHA);
        assert_eq!(sha.to_string(), SHA);
    }

    #[test]
    fn commit_sha_rejects_bad_input() {
        for bad in [
            "",
            "499bc8821c6b12b4e53c5bffcb21ec206f212d8",
            "499bc8821c6b12b4e53c5bffcb21ec206f212d81a",
            "499BC8821C6B12B4E53C5BFFCB21EC206F212D81",
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        ] {
            assert!(bad.parse::<CommitSha>().is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn repo_spec_split() {
        let spec = RepoSpec {
            kind: RepoKind::Model,
            repo: "ggml-org/models".to_string(),
        };
        assert_eq!(spec.split().unwrap(), ("ggml-org", "models"));

        for bad in [
            "",
            "ggml-org",
            "ggml-org/models/moved",
            "/models",
            "ggml-org/",
        ] {
            let spec = RepoSpec {
                kind: RepoKind::Model,
                repo: bad.to_string(),
            };
            assert!(spec.split().is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn file_prefixes_match_the_resolve_routes() {
        assert_eq!(RepoKind::Model.file_prefix(), "");
        assert_eq!(RepoKind::Dataset.file_prefix(), "datasets");
        assert_eq!(RepoKind::Space.file_prefix(), "spaces");
    }
}
