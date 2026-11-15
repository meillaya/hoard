//! `hoard` command-line interface.
//!
//! 0.1.0 ships the manifest and hashing commands that the artifact layer will
//! build on: `hash`, `manifest validate`, `manifest fmt`, `manifest hash`,
//! `manifest schema`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use hoard_core::Manifest;
use hoard_core::hash::{blake3_file, sha256_file};
use hoard_core::manifest_schema;

#[derive(Parser)]
#[command(
    name = "hoard",
    version,
    about = "Artifact pipeline for large model files: manifests, hashing, verification"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Hash a file: SHA-256 (source metadata) and BLAKE3 (content address).
    Hash {
        /// File to hash.
        path: PathBuf,
        /// Print JSON instead of labelled lines.
        #[arg(long)]
        json: bool,
    },
    /// Manifest v1 commands.
    Manifest {
        #[command(subcommand)]
        command: ManifestCommand,
    },
}

#[derive(Subcommand)]
enum ManifestCommand {
    /// Validate schema and semantics; `fmt --check` checks canonical form.
    Validate {
        /// Path to the manifest JSON.
        path: PathBuf,
    },
    /// Print the canonical form; `--check` fails if the file is not canonical.
    Fmt {
        /// Path to the manifest JSON.
        path: PathBuf,
        /// Exit non-zero when the file is not already canonical.
        #[arg(long)]
        check: bool,
    },
    /// Print the artifact ID: BLAKE3 over the canonical manifest bytes.
    Hash {
        /// Path to the manifest JSON.
        path: PathBuf,
    },
    /// Print the manifest JSON Schema (v1).
    Schema,
}

fn emit_stdout(bytes: &[u8]) -> Result<()> {
    use std::io::Write;

    let mut stdout = std::io::stdout().lock();
    match stdout.write_all(bytes).and_then(|()| stdout.flush()) {
        Ok(()) => Ok(()),
        // `hoard hash model.gguf | head` is normal usage.
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn emit_stderr(text: &str) {
    use std::io::Write;

    let _ = std::io::stderr().lock().write_all(text.as_bytes());
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            emit_stderr(&format!("error: {error:#}\n"));
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Hash { path, json } => hash_command(&path, json),
        Command::Manifest { command } => match command {
            ManifestCommand::Validate { path } => {
                Manifest::from_path(&path)
                    .with_context(|| format!("failed to read {}", path.display()))?
                    .validate()
                    .with_context(|| format!("invalid manifest {}", path.display()))?;
                Ok(())
            }
            ManifestCommand::Fmt { path, check } => {
                let manifest = Manifest::from_path(&path)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                let canonical = manifest.canonical_bytes();
                if check {
                    let current = std::fs::read(&path)
                        .with_context(|| format!("failed to read {}", path.display()))?;
                    if current != canonical {
                        bail!(
                            "{} is not canonical; run `hoard manifest fmt {}` and redirect the output",
                            path.display(),
                            path.display()
                        );
                    }
                    return Ok(());
                }
                emit_stdout(&canonical)
            }
            ManifestCommand::Hash { path } => {
                let manifest = Manifest::from_path(&path)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                manifest
                    .validate()
                    .with_context(|| format!("invalid manifest {}", path.display()))?;
                emit_stdout(format!("{}\n", manifest.hash()).as_bytes())
            }
            ManifestCommand::Schema => {
                let mut text = serde_json::to_string_pretty(&manifest_schema())?;
                text.push('\n');
                emit_stdout(text.as_bytes())
            }
        },
    }
}

fn hash_command(path: &Path, json: bool) -> Result<()> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if !metadata.is_file() {
        bail!("{} is not a regular file", path.display());
    }
    let size = metadata.len();
    let sha256 = sha256_file(path).with_context(|| format!("failed to hash {}", path.display()))?;
    let blake3 = blake3_file(path).with_context(|| format!("failed to hash {}", path.display()))?;

    let mut text = if json {
        let value = serde_json::json!({
            "schema": 1,
            "file": path.display().to_string(),
            "size": size,
            "sha256": sha256.to_string(),
            "blake3": blake3.to_string(),
        });
        serde_json::to_string_pretty(&value)?
    } else {
        format!(
            "{}\n  size    {size}\n  sha256  {sha256}\n  blake3  {blake3}",
            path.display()
        )
    };
    text.push('\n');
    emit_stdout(text.as_bytes())
}
