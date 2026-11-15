//! `hoard` command-line interface.
//!
//! Commands: `hash`, `manifest`, `fetch`, `verify`, `store`, and `push`.

use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use hoard_core::artifact::{self, Selection, SourceFile};
use hoard_core::cas::{self, Entry, Store};
use hoard_core::hash::{blake3_file, sha256_file};
use hoard_core::{
    FileEntry, Hash32, Manifest, RepoType, Role, Source, SourceKind, manifest_schema,
};
use hoard_hf::{CommitSha, RemoteFile, RepoKind, RepoSpec, Transport};

#[derive(Parser)]
#[command(
    name = "hoard",
    version,
    about = "Artifact pipeline for large model files: manifests, hashing, and verified fetch"
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
    /// Download a Hugging Face artifact set, verify it, and write a manifest.
    Fetch(FetchArgs),
    /// Verify a manifest against a local directory or the CAS.
    Verify(VerifyArgs),
    /// Add the manifest files to the Nix store as flat fixed-output paths.
    Store(StoreArgs),
    /// Push the manifest files to a Nix binary cache.
    Push(PushArgs),
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

#[derive(ValueEnum, Clone, Copy, Debug)]
enum RepoKindArg {
    Model,
    Dataset,
    Space,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum RoleArg {
    Weights,
    Mmproj,
    Mtp,
    Tokenizer,
    Config,
    Other,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum TransportArg {
    Auto,
    Xet,
    Http,
}

#[derive(clap::Args)]
struct FetchArgs {
    /// Repository and revision: `hf:owner/name[@rev]`.
    #[arg(conflicts_with = "manifest")]
    spec: Option<String>,

    /// Fetch the files listed in an existing manifest instead of a spec.
    #[arg(long, value_name = "FILE", conflicts_with = "spec")]
    manifest: Option<PathBuf>,

    /// Repository file to fetch. Repeat for several files. Required with a spec.
    #[arg(long = "file", value_name = "PATH")]
    files: Vec<String>,

    /// Group written to the weights entries in a new manifest.
    #[arg(long)]
    group: Option<String>,

    /// Repository kind for a spec.
    #[arg(long, value_enum, default_value_t = RepoKindArg::Model)]
    repo_type: RepoKindArg,

    /// Role override for every selected file in a new manifest.
    #[arg(long, value_enum)]
    role: Option<RoleArg>,

    /// Write the generated manifest to this path. Required with a spec.
    #[arg(long)]
    manifest_out: Option<PathBuf>,

    /// Materialize files under this directory, or to this file when one file is selected.
    #[arg(long)]
    out: Option<PathBuf>,

    /// Transport to use for downloads.
    #[arg(long, value_enum, default_value_t = TransportArg::Auto)]
    transport: TransportArg,

    /// Use only the CAS. Fail if a file is missing.
    #[arg(long)]
    offline: bool,
}

#[derive(clap::Args)]
struct VerifyArgs {
    /// Manifest to verify.
    #[arg(long)]
    manifest: PathBuf,

    /// Verify files under this directory instead of the CAS.
    #[arg(long, conflicts_with = "store")]
    dir: Option<PathBuf>,

    /// Verify the CAS entries (the default).
    #[arg(long)]
    store: bool,
}

#[derive(clap::Args)]
struct StoreArgs {
    /// Manifest whose files should be added to the Nix store.
    #[arg(long)]
    manifest: PathBuf,
}

#[derive(clap::Args)]
struct PushArgs {
    /// Manifest whose files should be pushed.
    #[arg(long)]
    manifest: PathBuf,

    /// Nix store URI, for example `file:///tmp/cache` or an Attic cache.
    #[arg(long)]
    to: String,
}

fn emit_stdout(bytes: &[u8]) -> Result<()> {
    use std::io::Write;

    let mut stdout = std::io::stdout().lock();
    match stdout.write_all(bytes).and_then(|()| stdout.flush()) {
        Ok(()) => Ok(()),
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
                load_manifest(&path)?;
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
                let manifest = load_manifest(&path)?;
                emit_stdout(format!("{}\n", manifest.hash()).as_bytes())
            }
            ManifestCommand::Schema => {
                let mut text = serde_json::to_string_pretty(&manifest_schema())?;
                text.push('\n');
                emit_stdout(text.as_bytes())
            }
        },
        Command::Fetch(args) => fetch_command(args),
        Command::Verify(args) => verify_command(args),
        Command::Store(args) => store_command(&args.manifest),
        Command::Push(args) => push_command(&args.manifest, &args.to),
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

fn load_manifest(path: &Path) -> Result<Manifest> {
    let manifest =
        Manifest::from_path(path).with_context(|| format!("failed to read {}", path.display()))?;
    manifest
        .validate()
        .with_context(|| format!("invalid manifest {}", path.display()))?;
    Ok(manifest)
}

fn repo_kind(kind: RepoKindArg) -> RepoKind {
    match kind {
        RepoKindArg::Model => RepoKind::Model,
        RepoKindArg::Dataset => RepoKind::Dataset,
        RepoKindArg::Space => RepoKind::Space,
    }
}

fn repo_type(kind: RepoKindArg) -> RepoType {
    match kind {
        RepoKindArg::Model => RepoType::Model,
        RepoKindArg::Dataset => RepoType::Dataset,
        RepoKindArg::Space => RepoType::Space,
    }
}

fn role_from_arg(role: RoleArg) -> Role {
    match role {
        RoleArg::Weights => Role::Weights,
        RoleArg::Mmproj => Role::Mmproj,
        RoleArg::Mtp => Role::Mtp,
        RoleArg::Tokenizer => Role::Tokenizer,
        RoleArg::Config => Role::Config,
        RoleArg::Other => Role::Other,
    }
}

fn transport_from_arg(transport: TransportArg) -> Transport {
    match transport {
        TransportArg::Auto => Transport::Auto,
        TransportArg::Xet => Transport::Xet,
        TransportArg::Http => Transport::Http,
    }
}

fn parse_spec(input: &str, kind: RepoKindArg) -> Result<(RepoSpec, String)> {
    let rest = input
        .strip_prefix("hf:")
        .with_context(|| format!("invalid spec {input:?}; expected hf:owner/name[@rev]"))?;
    let (repo, rev) = match rest.rsplit_once('@') {
        Some((repo, rev)) if !rev.is_empty() => (repo, rev.to_owned()),
        Some(_) => bail!("empty revision in spec {input:?}"),
        None => (rest, "main".to_owned()),
    };
    let mut parts = repo.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        bail!("invalid repository {repo:?}; expected owner/name");
    };
    if owner.is_empty() || name.is_empty() {
        bail!("invalid repository {repo:?}; expected owner/name");
    }

    Ok((
        RepoSpec {
            kind: repo_kind(kind),
            repo: repo.to_owned(),
        },
        rev,
    ))
}

fn sha256_from_remote(file: &RemoteFile) -> Result<Hash32> {
    let text = file.sha256.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "{} has no LFS sha256; M1 fetch supports LFS files only",
            file.path
        )
    })?;
    text.parse()
        .map_err(|error| anyhow::anyhow!("invalid sha256 for {}: {error}", file.path))
}

fn source_file(file: &RemoteFile) -> Result<SourceFile> {
    Ok(SourceFile {
        path: file.path.clone(),
        size: file.size,
        sha256: sha256_from_remote(file)?,
    })
}

fn find_remote<'a>(remote: &'a [RemoteFile], path: &str) -> Result<&'a RemoteFile> {
    remote
        .iter()
        .find(|file| file.path == path)
        .ok_or_else(|| anyhow::anyhow!("{path:?} is not in the repository tree"))
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    Ok(())
}

fn fetch_command(args: FetchArgs) -> Result<()> {
    match (&args.spec, &args.manifest) {
        (Some(_), Some(_)) => bail!("use either a spec or --manifest, not both"),
        (None, None) => bail!("provide a spec or --manifest"),
        (Some(spec), None) => fetch_from_spec(spec, &args),
        (None, Some(manifest)) => fetch_from_manifest(manifest, &args),
    }
}

fn fetch_from_spec(spec_text: &str, args: &FetchArgs) -> Result<()> {
    if args.offline {
        bail!("--offline needs --manifest; a spec has to be resolved online");
    }
    if args.files.is_empty() {
        bail!("fetch from a spec needs at least one --file");
    }

    let (spec, revision) = parse_spec(spec_text, args.repo_type)?;
    let client = hoard_hf::client_from_env().context("failed to create Hugging Face client")?;
    let rev = client
        .resolve(&spec, &revision)
        .with_context(|| format!("failed to resolve {}", spec.repo))?;
    let remote = client
        .list(&spec, &rev)
        .with_context(|| format!("failed to list {}", spec.repo))?;

    let selected: Vec<RemoteFile> = args
        .files
        .iter()
        .map(|path| find_remote(&remote, path).cloned())
        .collect::<Result<_>>()?;
    let sources: Vec<SourceFile> = selected.iter().map(source_file).collect::<Result<_>>()?;

    let store = Store::open(cas::store_root_from_env()).context("failed to open the CAS")?;
    let mut fetched: Vec<(String, Entry)> = Vec::with_capacity(selected.len());
    for file in &selected {
        let sha256 = sha256_from_remote(file)?;
        let temp_dir = tempfile::Builder::new()
            .prefix("hoard-fetch-")
            .tempdir_in(store.root().join("tmp"))
            .context("failed to create a temporary download directory")?;
        let destination = temp_dir.path().join("download");
        let stats = client
            .download(
                &spec,
                &rev,
                file,
                &destination,
                transport_from_arg(args.transport),
            )
            .with_context(|| format!("failed to download {}", file.path))?;
        let entry = store
            .put_file(&destination, sha256)
            .with_context(|| format!("failed to store {}", file.path))?;
        eprintln!(
            "fetched {} ({} bytes, {:?}, {:.2}s)",
            file.path,
            stats.bytes,
            stats.transport,
            stats.elapsed.as_secs_f64()
        );
        fetched.push((file.path.clone(), entry));
    }

    let manifest = artifact::build_manifest(
        Source {
            kind: SourceKind::HuggingFace,
            repo: spec.repo.clone(),
            revision: rev.as_str().to_owned(),
            repo_type: repo_type(args.repo_type),
        },
        &sources,
        &fetched,
        &Selection {
            files: args.files.clone(),
            group: args.group.clone(),
            role: args.role.map(role_from_arg),
        },
    )
    .context("failed to build the manifest")?;

    let manifest_out = args
        .manifest_out
        .as_deref()
        .context("--manifest-out is required when fetching from a spec")?;
    ensure_parent(manifest_out)?;
    artifact::write_canonical(&manifest, manifest_out)
        .with_context(|| format!("failed to write {}", manifest_out.display()))?;
    eprintln!("wrote {} ({})", manifest_out.display(), manifest.hash());

    if let Some(out) = &args.out {
        materialize(&store, &fetched, out, false)?;
    }

    Ok(())
}

fn fetch_from_manifest(manifest_path: &Path, args: &FetchArgs) -> Result<()> {
    if args.manifest_out.is_some() {
        bail!("--manifest-out cannot be combined with --manifest");
    }
    let manifest = load_manifest(manifest_path)?;
    let files = select_manifest_files(&manifest, &args.files)?;

    let store = Store::open(cas::store_root_from_env()).context("failed to open the CAS")?;
    let mut fetched: Vec<(String, Entry)> = Vec::with_capacity(files.len());
    let mut missing: Vec<&FileEntry> = Vec::new();

    for file in &files {
        let entry = Entry {
            blake3: file.blake3,
            sha256: file.sha256,
            size: file.size,
        };
        if store.contains(&file.blake3) {
            store
                .verify(&entry)
                .with_context(|| format!("corrupt CAS entry for {}", file.path))?;
            fetched.push((file.path.clone(), entry));
        } else {
            missing.push(file);
        }
    }

    if !missing.is_empty() {
        if args.offline {
            let paths: Vec<&str> = missing.iter().map(|file| file.path.as_str()).collect();
            bail!("missing from the CAS and --offline is set: {paths:?}");
        }

        let spec = RepoSpec {
            kind: match manifest.source.repo_type {
                RepoType::Model => RepoKind::Model,
                RepoType::Dataset => RepoKind::Dataset,
                RepoType::Space => RepoKind::Space,
            },
            repo: manifest.source.repo.clone(),
        };
        let rev: CommitSha = manifest
            .source
            .revision
            .parse()
            .with_context(|| format!("invalid revision {:?}", manifest.source.revision))?;
        let client = hoard_hf::client_from_env().context("failed to create Hugging Face client")?;
        let remote = client
            .list(&spec, &rev)
            .with_context(|| format!("failed to list {}", spec.repo))?;

        for file in missing {
            let remote_file = find_remote(&remote, &file.path)?;
            if remote_file.size != file.size || sha256_from_remote(remote_file)? != file.sha256 {
                bail!(
                    "source metadata for {} does not match the manifest",
                    file.path
                );
            }
            let temp_dir = tempfile::Builder::new()
                .prefix("hoard-fetch-")
                .tempdir_in(store.root().join("tmp"))
                .context("failed to create a temporary download directory")?;
            let destination = temp_dir.path().join("download");
            let stats = client
                .download(
                    &spec,
                    &rev,
                    remote_file,
                    &destination,
                    transport_from_arg(args.transport),
                )
                .with_context(|| format!("failed to download {}", file.path))?;
            let entry = store
                .put_file(&destination, file.sha256)
                .with_context(|| format!("failed to store {}", file.path))?;
            if entry.blake3 != file.blake3 {
                bail!(
                    "blake3 mismatch for {}: manifest {}, downloaded {}",
                    file.path,
                    file.blake3,
                    entry.blake3
                );
            }
            eprintln!(
                "fetched {} ({} bytes, {:?}, {:.2}s)",
                file.path,
                stats.bytes,
                stats.transport,
                stats.elapsed.as_secs_f64()
            );
            fetched.push((file.path.clone(), entry));
        }
    }

    if let Some(out) = &args.out {
        let single = args.files.len() == 1 && !out.is_dir();
        materialize(&store, &fetched, out, single)?;
    }
    Ok(())
}

fn select_manifest_files<'a>(
    manifest: &'a Manifest,
    requested: &[String],
) -> Result<Vec<&'a FileEntry>> {
    if requested.is_empty() {
        return Ok(manifest.files.iter().collect());
    }
    requested
        .iter()
        .map(|path| {
            manifest
                .files
                .iter()
                .find(|file| &file.path == path)
                .ok_or_else(|| anyhow::anyhow!("{path:?} is not in the manifest"))
        })
        .collect()
}

fn materialize(store: &Store, entries: &[(String, Entry)], out: &Path, single: bool) -> Result<()> {
    if single {
        if entries.len() != 1 {
            bail!("--out as a file needs exactly one selected file");
        }
        ensure_parent(out)?;
        store
            .materialize(&entries[0].1, out)
            .with_context(|| format!("failed to write {}", out.display()))?;
        return Ok(());
    }

    for (path, entry) in entries {
        let destination = out.join(path);
        ensure_parent(&destination)?;
        store
            .materialize(entry, &destination)
            .with_context(|| format!("failed to write {}", destination.display()))?;
    }
    Ok(())
}

fn verify_command(args: VerifyArgs) -> Result<()> {
    let manifest = load_manifest(&args.manifest)?;
    let store = if args.dir.is_none() {
        Some(Store::open(cas::store_root_from_env()).context("failed to open the CAS")?)
    } else {
        None
    };

    let mut failures = 0usize;
    for file in &manifest.files {
        let result = match (&args.dir, &store) {
            (Some(dir), _) => verify_in_dir(dir, file),
            (None, Some(store)) => verify_in_store(store, file),
            (None, None) => unreachable!("verify always has a source"),
        };
        if let Err(error) = result {
            failures += 1;
            emit_stderr(&format!("{}: {error}\n", file.path));
        }
    }

    if failures > 0 {
        bail!("{failures} file(s) failed verification");
    }
    eprintln!("verified {} file(s)", manifest.files.len());
    Ok(())
}

fn verify_in_dir(dir: &Path, file: &FileEntry) -> Result<()> {
    let path = dir.join(&file.path);
    let metadata =
        std::fs::metadata(&path).with_context(|| format!("failed to stat {}", path.display()))?;
    if !metadata.is_file() {
        bail!("{} is not a regular file", path.display());
    }
    if metadata.len() != file.size {
        bail!(
            "size mismatch: expected {}, found {}",
            file.size,
            metadata.len()
        );
    }
    let sha256 =
        sha256_file(&path).with_context(|| format!("failed to hash {}", path.display()))?;
    if sha256 != file.sha256 {
        bail!("sha256 mismatch: expected {}, found {sha256}", file.sha256);
    }
    let blake3 =
        blake3_file(&path).with_context(|| format!("failed to hash {}", path.display()))?;
    if blake3 != file.blake3 {
        bail!("blake3 mismatch: expected {}, found {blake3}", file.blake3);
    }
    Ok(())
}

fn verify_in_store(store: &Store, file: &FileEntry) -> Result<()> {
    let entry = Entry {
        blake3: file.blake3,
        sha256: file.sha256,
        size: file.size,
    };
    Ok(store.verify(&entry)?)
}

fn store_command(manifest_path: &Path) -> Result<()> {
    let manifest = load_manifest(manifest_path)?;
    let store = Store::open(cas::store_root_from_env()).context("failed to open the CAS")?;
    let paths = nix_store_paths(&store, &manifest)?;
    for (path, store_path) in paths {
        emit_stdout(format!("{path}\t{store_path}\n").as_bytes())?;
    }
    Ok(())
}

fn push_command(manifest_path: &Path, to: &str) -> Result<()> {
    let manifest = load_manifest(manifest_path)?;
    let store = Store::open(cas::store_root_from_env()).context("failed to open the CAS")?;
    let paths = nix_store_paths(&store, &manifest)?;

    let mut command = ProcessCommand::new("nix");
    command.args(["copy", "--to", to]);
    for (_, store_path) in &paths {
        command.arg(store_path);
    }
    let status = command
        .status()
        .context("failed to run `nix copy`; is Nix on PATH?")?;
    if !status.success() {
        bail!("nix copy failed with {status}");
    }
    eprintln!("pushed {} path(s) to {to}", paths.len());
    Ok(())
}

fn nix_store_paths(store: &Store, manifest: &Manifest) -> Result<Vec<(String, String)>> {
    let mut paths = Vec::with_capacity(manifest.files.len());
    for file in &manifest.files {
        let entry = Entry {
            blake3: file.blake3,
            sha256: file.sha256,
            size: file.size,
        };
        store
            .verify(&entry)
            .with_context(|| format!("missing or corrupt CAS entry for {}", file.path))?;

        let sha_hex = file.sha256.to_string();
        let base = file
            .path
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .with_context(|| format!("invalid path {:?}", file.path))?;
        let name = format!("hoard-{}-{}", &sha_hex[..12], base);

        let temp_dir = tempfile::Builder::new()
            .prefix("hoard-nix-")
            .tempdir_in(store.root().join("tmp"))
            .context("failed to create a temporary directory")?;
        let temp_path = temp_dir.path().join(&name);
        let blob = store.path_for(&entry.blake3);
        if std::fs::hard_link(&blob, &temp_path).is_err() {
            std::fs::copy(&blob, &temp_path)
                .with_context(|| format!("failed to prepare {} for the Nix store", file.path))?;
        }

        let output = ProcessCommand::new("nix-store")
            .args(["--add-fixed", "sha256"])
            .arg(&temp_path)
            .output()
            .context("failed to run `nix-store --add-fixed`; is Nix on PATH?")?;
        if !output.status.success() {
            bail!(
                "nix-store --add-fixed failed for {}: {}",
                file.path,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let store_path = String::from_utf8_lossy(&output.stdout)
            .lines()
            .last()
            .unwrap_or_default()
            .trim()
            .to_owned();
        if store_path.is_empty() {
            bail!("nix-store printed no store path for {}", file.path);
        }
        paths.push((file.path.clone(), store_path));
    }
    Ok(paths)
}
