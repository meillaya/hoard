# Fuzz targets for `hoard-gguf`

`cargo-fuzz` harnesses for the GGUF parser and the inspect JSON contract. The
fuzzers are an independent Cargo workspace (`fuzz/Cargo.toml` has its own
`[workspace]` table), so they never affect `cargo test --workspace` and are not
built by the normal test run.

## Targets

| Target | What it must guarantee |
| --- | --- |
| `parse_gguf` | For arbitrary bytes, `Gguf::parse` returns a typed error or a validated view. Accepted files must survive every accessor and full `inspect` report generation without panicking or hanging. |
| `manifest_or_inspect_json` | For every accepted file, the inspect report serializes to valid JSON (`schema: 1`) for both pretty and compact output, round-trips to an equivalent document, and the generated JSON Schema stays constructible. |

Neither target weakens parser caps or panic behaviour; a panic is a finding.

## Requirements

`cargo-fuzz` drives libFuzzer through Rust nightly features (`-Z sanitizer`), so
**a nightly toolchain is required**. The main repository pins stable Rust in
`rust-toolchain.toml`; install nightly next to it:

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz
```

If you manage toolchains outside `~/.cargo` (for example in a sandbox or Nix
shell), point `RUSTUP_HOME`/`CARGO_HOME` at writable directories and put the
toolchain and `cargo-fuzz` binaries on `PATH`.

## Commands

Run from the repository root (Cargo finds the fuzz workspace by `--manifest-path`,
or simply `cd fuzz` first):

```sh
CARGO_HOME=$PWD/.cargo-home cargo +nightly fuzz build

# Open-ended campaigns (Ctrl-C to stop; corpus grows in fuzz/corpus/<target>/).
cargo +nightly fuzz run parse_gguf
cargo +nightly fuzz run manifest_or_inspect_json

# Short campaigns, appropriate for a local pre-merge gate.
cargo +nightly fuzz run parse_gguf -- -max_total_time=60
cargo +nightly fuzz run manifest_or_inspect_json -- -max_total_time=30

# Reproduce a crash from fuzz/artifacts/<target>/<artifact>.
cargo +nightly fuzz run parse_gguf fuzz/artifacts/parse_gguf/crash-<hash>
```

`cargo fuzz run` also accepts extra corpus directories, for example
`cargo +nightly fuzz run manifest_or_inspect_json fuzz/corpus/parse_gguf` to
reuse the parser seeds.

## Seed corpus

`fuzz/corpus/parse_gguf/` and `fuzz/corpus/manifest_or_inspect_json/` are
committed and contain:

- copies of the committed synthetic fixtures (`synthetic-minimal.gguf`,
  `synthetic-rich.gguf`), and
- tiny malformed samples generated from the minimal fixture (bad magic, bad and
  big-endian versions, truncated header, header-only file, non-zero padding,
  empty input).

Regenerate the corpus after the synthetic fixtures change:

```sh
bash fuzz/sync-corpus.sh
```

Do not commit `fuzz/target/`, `fuzz/artifacts/`, or coverage output; they are
gitignored.

## Without nightly

Nightly is genuinely required to *run* the targets. Without it, the most that
can be validated is compilation of the fuzz crate and its targets:

```sh
# Stable toolchain: compiles the harness code and the libfuzzer-sys runtime,
# but cannot run a sanitizer campaign.
cargo check --manifest-path fuzz/Cargo.toml
```

Continuous integration can run short campaigns on a nightly toolchain when one
is available (for example a scheduled job); the regular test suite runs the
deterministic `proptest` checks in `crates/hoard-gguf/tests/robustness.rs`
instead.
