# nixpkgs packaging draft for hoard

Status: **critical draft, all automated checks green.** This directory holds the
package expression and the review material for adding
[`hoard`](https://github.com/meillaya/hoard) to nixpkgs as `hoard-artifact`.
The draft pull request is
<https://github.com/NixOS/nixpkgs/pull/588800>. The PR is rebased on current
`master`, squashed to one commit, includes the `meillaya` maintainer entry, and
builds with `nixpkgs-review`. It remains a draft pending reviewer sign-off.

## Why the attribute is not called `hoard`

nixpkgs already has a top-level `hoard` package: `pkgs/by-name/ho/hoard/package.nix`
is [Hyde46/hoard](https://github.com/Hyde46/hoard), a CLI command organizer
(version 1.4.2). The by-name directory and attribute name must therefore differ
for this package. The draft uses:

| Field | Value |
| --- | --- |
| nixpkgs attribute / by-name directory | `hoard-artifact` (planned: `pkgs/by-name/ho/hoard-artifact/package.nix`) |
| `pname` in this draft | `hoard` (upstream project name; reviewer may prefer `hoard-artifact` for consistency) |
| installed binaries | `hoard`, `hoard-gguf` |
| version | `0.1.0` |

`hoard-artifact` is the name the upstream workspace is moving the artifact CLI
crate to (`hoard-artifact`), which makes it a natural disambiguator. The binary
names do not change.

## Expression

`package.nix` is a `buildRustPackage` expression with:

- `fetchFromGitHub` pinned to `meillaya/hoard` at tag `v0.1.0`.
- `hash` and `cargoHash` computed from a clean build (see below). No
  `cargoLock`/`importCargoLock`; a plain cargo vendor hash so the package
  follows the common nixpkgs Rust pattern.
- `cargoBuildFlags = [ "--workspace" ]`, `cargoTestFlags = [ "--workspace" ]`,
  and the default cargo install hook, which installs both `hoard` and
  `hoard-gguf` from `target/release`.
- `nativeBuildInputs = [ pkg-config ]`, `buildInputs = [ aws-lc ]`: the
  dependency tree enables `aws-lc-rs` through `rustls`, and nixpkgs' Rust
  setup hook exports `AWS_LC_SYS_USE_SYSTEM=1`, so the system `aws-lc` is used
  instead of the crate's vendored C build.
- A `preCheck` that exports `SSL_CERT_FILE` and `SSL_CERT_DIR` pointing at
  `cacert`: the Hub-client tests build a `reqwest` client with the system root
  store, and the Nix sandbox has no `/etc/ssl/certs`.
- `doCheck` is left at the default (`true`), so the workspace test suite runs
  inside the Nix sandbox. Tests that need the downloaded real GGUF corpus
  (`fixtures/real/`, gitignored and therefore absent from the source tarball)
  skip themselves; the ignored live Xet gate is not run.
- `meta.license = asl20`, `meta.mainProgram = "hoard"`,
  `meta.platforms = lib.platforms.unix`, `meta.homepage`, `meta.changelog`.
- `passthru.updateScript = nix-update-script { }` (currently ineffective
  because upstream has no tags; see caveats).

## Computed hashes

| Hash | Value | How obtained |
| --- | --- | --- |
| `src.hash` | `sha256-cJka6IG5DIKAX4EPMeAUBt51qT9Ov3Pb7FLX3bI3cGk=` | `lib.fakeHash` build against nixpkgs `d5cc683…`; reported `got:` value |
| `cargoHash` | `sha256-z0B467EbW+XYK6zmzVuNdX/VpPUzzVI43d+DJIqa8DM=` | `lib.fakeHash` build against nixpkgs `d5cc683…`; reported `got:` value |

`src.hash` is the NAR hash of the GitHub tarball for tag `v0.1.0`; `cargoHash`
is the hash of the vendored crates from the committed `Cargo.lock`. Both were
recomputed after the CLI crate rename and verified with a clean `nix-build` of
the final expression.

## Verification

Environment: Determinate Nix 3.24.1 (`nix 2.35.3`), `x86_64-linux`, nixpkgs
pinned to `NixOS/nixpkgs` rev `d5cc6833999746cffdfe0598f7bc801a57b887a9`
(the project's `flake.lock` pin), rustc 1.99.1 / cargo 1.99.1 from nixpkgs.

The expression was evaluated and built against that exact nixpkgs checkout
without using the project flake. The final run used tag `v0.1.0` and the hashes
in the table above:

```sh
NP=$(nix eval --impure --raw --expr 'builtins.toString (builtins.fetchTree {
  type = "github"; owner = "NixOS"; repo = "nixpkgs";
  rev = "d5cc6833999746cffdfe0598f7bc801a57b887a9";
  narHash = "sha256-Wpz/7x/YWnDIBu7yj6ixe8j6fIMsidd9Y+o1PqPF3iM=";
})')

XDG_CACHE_HOME=$PWD/.cache-nix \
  nix-build --out-link .scratch/nixpkgs-hoard/result -E \
  "with import $NP {}; callPackage ./packaging/nixpkgs/package.nix {}"
```

Result and checks (from the final clean build):

- `nix-build` exit code: `0`
- output: `/nix/store/7njwlnvn7am8kh3c9nmiswnpfm2xqmfc-hoard-0.1.0`
- installed binaries: `hoard` (14.9 MB), `hoard-gguf` (1.7 MB)
- `result/bin/hoard --version`: `hoard 0.1.0`
- `result/bin/hoard-gguf --version`: `hoard-gguf 0.1.0`
- `result/bin/hoard-gguf inspect <synthetic-rich.gguf>` (both binary and
  fixture taken from the packaged source): reports GGUF v3, `llama`,
  alignment 64, 3 tensors, split shard 1 of 2, and the F32/Q4_K/Q8_0 quant mix;
  `--json` gives `schema: 1`, `tensors.count: 3`, `split: {no: 1, count: 2,
  tensors_count: 48}`.
- `result/bin/hoard manifest hash <stories260k.json>`:
  `blake3:56bbfeba5585a228ce8ac25e41d2fba1b16f80e837f93257b61d8fda11bb2421`;
  `manifest validate` and `manifest fmt --check` exit 0.
- `result/bin/hoard hash <synthetic-minimal.gguf>`: prints SHA-256 and BLAKE3.
- test suite in the sandbox: `checkPhase` completed in 2m20s with
  **336 passed, 0 failed, 2 ignored** across 25 test binaries/suites. The two
  ignored tests are the upstream `#[ignore]`d live Hugging Face/Xet gate and a
  helper-spawned test that is run separately. No test failed, and no test
  attempted real network access beyond loopback (the `wiremock` Hub suite
  passed in the sandbox).
- linkage confirms the system library is used:
  `ldd result/bin/hoard` → `libcrypto-awslc.so.1 => /nix/store/…-aws-lc-5.12.0/lib/libcrypto-awslc.so.1`.
- cross-system evaluation only: the derivation path evaluates for
  `x86_64-linux`, `aarch64-linux`, and `aarch64-darwin`; only `x86_64-linux`
  was built here.
- top-level attribute: evaluating the expression through an overlay as
  `pkgs.hoard-artifact` builds the same store path, so the expression works as
  a by-name package body.
- `nixfmt --check packaging/nixpkgs/package.nix`: clean.

`nixpkgs-review` was run on the final revision `eca10a66` from a nixpkgs
checkout. Result: 1 package built on `x86_64-linux` (`hoard-artifact`).

## Opened draft PR

**Commit message**

```text
hoard-artifact: init at 0.1.0
```

**PR title**

```text
hoard-artifact: init at 0.1.0
```

**PR body (draft)**

```markdown
###### Description of changes

Adds `hoard`, a Rust workspace for verifiable, content-addressed model
artifacts, as the new top-level attribute `hoard-artifact` (the existing
`hoard` attribute is Hyde46/hoard, a different project).

The package installs two binaries:

- `hoard`: artifact CLI — `hash`, `manifest`, `fetch`, `verify`, `store`,
  `push`.
- `hoard-gguf`: strict GGUF parser and inspector (`inspect`, `schema`).

Upstream: https://github.com/meillaya/hoard (Apache-2.0, Rust 1.99 edition
2024, Cargo.lock committed).

`buildRustPackage` builds the whole workspace and installs both binaries. The
dependency tree enables `aws-lc-rs` via `rustls`, so the expression passes
`pkg-config` and `aws-lc` to match nixpkgs' `AWS_LC_SYS_USE_SYSTEM=1`; a
`preCheck` points `SSL_CERT_FILE`/`SSL_CERT_DIR` at `cacert` for the HTTP
client tests, which load the system root store. Tests run by default
(`doCheck = true`); tests that
need the gitignored real GGUF corpus skip when it is absent, and the live Xet
test is `#[ignore]`d upstream.

Tested with `nix-build` against nixpkgs at the project's `flake.lock` pin
(`d5cc6833999746cffdfe0598f7bc801a57b887a9`, 2026-11-13). Only
`x86_64-linux` was built here.

###### Things done

- [x] Built on platform(s): `x86_64-linux`
- [x] Tested via `nix-build` of the expression against pinned nixpkgs
- [x] Ran `hoard --version`, `hoard-gguf --version`, `hoard-gguf inspect`
- [x] Test suite ran in the Nix sandbox (`doCheck`): 336 passed, 0 failed,
      2 ignored (336 total across 25 suites)
- [ ] `nixpkgs-review` run: not installed in this environment
- [ ] `sandbox = true`-clean on aarch64/darwin: not verified (cross-platform)
- [ ] Tested with a nixpkgs maintainer against current `master`

###### Open questions for reviewers

1. Attribute name: `hoard-artifact` (dir/attr) with `pname = "hoard"`, or
   rename `pname` to `hoard-artifact`? Upstream is renaming the CLI crate to
   `hoard-artifact`; `hoard` is taken.
2. Upstream has no git tags yet. Should this wait for a `v0.1.0` tag so
   `fetchFromGitHub { rev = "v${version}"; }` and `nix-update-script` work,
   or should it use the commit hash and the `0.1.0-unstable-YYYY-MM-DD`
   version convention?
3. A maintainer entry: upstream has no nixpkgs maintainer yet; `maintainers`
   is intentionally empty until one is added.
```

## Honest caveats and blockers

1. **Maintainer.** Resolved in the same PR: `maintainers/maintainer-list.nix`
   adds `meillaya` and the package sets
   `maintainers = [ lib.maintainers.meillaya ]`. Reviewers can ask to split the
   maintainer entry into a separate PR.
2. **Release tag.** Resolved: upstream tagged `v0.1.0`, and the expression now
   pins that tag. `nix-update-script` can track future tags.
3. **Attribute name collision.** `hoard` exists in nixpkgs
   (Hyde46/hoard 1.4.2). The natural attribute is `hoard-artifact`; whether
   `pname` should also be `hoard-artifact` is a reviewer call. There are
   ~1400 by-name packages whose `pname` differs from the attribute name, so it
   is not unprecedented, but it is worth asking.
4. **AWS-LC system library.** nixpkgs' Rust setup hook already exports
   `AWS_LC_SYS_USE_SYSTEM=1` and `default-crate-overrides.nix` adds
   `pkg-config`/`aws-lc` for `aws-lc-sys` >= 0.42.0. The explicit
   `nativeBuildInputs`/`buildInputs` in this expression are therefore
   belt-and-braces (and document the requirement); reviewers may want them
   trimmed if the override is considered sufficient.
5. **CA bundle handled in `preCheck`.** The client tests construct a
   `reqwest` client backed by `rustls-platform-verifier`, which reads the
   system CA store. The sandbox has no `/etc/ssl/certs`, hence the
   `SSL_CERT_FILE`/`SSL_CERT_DIR` exports. If a future dependency change makes
   the client lazy, the exports can be scoped more tightly; they are cheap and
   harmless today.
6. **Tests.** `doCheck = true` and the suite runs in the Nix sandbox. Real
   corpus tests skip (the corpus is gitignored); the live Hugging Face/Xet test
   is ignored upstream. Mock Hub tests use `wiremock` on loopback. Nothing in
   the build downloads model bytes.
7. **Platform coverage.** Only `x86_64-linux` was built in this environment.
   `meta.platforms = lib.platforms.unix` is the upstream support statement
   (Linux and aarch64-darwin flake outputs); aarch64-linux/aarch64-darwin and
   Darwin's `aws-lc` details are unverified here and should be checked by
   builders or reviewers.
8. **Expression location.** The file is at `packaging/nixpkgs/package.nix` in
   the upstream repo for review. In the nixpkgs PR it must be moved to
   `pkgs/by-name/ho/hoard-artifact/package.nix` (if that attribute name is
   chosen) and committed on a nixpkgs fork. No nixpkgs checkout was modified and
   no PR was opened from this task.
9. **No `nixpkgs-review`.** Not installed in this environment; only a direct
   `nix-build` against the pinned nixpkgs revision was possible.

## Reproducing

```sh
cd /path/to/hoard
NP=$(nix eval --impure --raw --expr 'builtins.toString (builtins.fetchTree {
  type = "github"; owner = "NixOS"; repo = "nixpkgs";
  rev = "d5cc6833999746cffdfe0598f7bc801a57b887a9";
  narHash = "sha256-Wpz/7x/YWnDIBu7yj6ixe8j6fIMsidd9Y+o1PqPF3iM=";
})')
XDG_CACHE_HOME=$PWD/.cache-nix \
  nix-build --out-link .scratch/nixpkgs-hoard/result -E \
  "with import $NP {}; callPackage ./packaging/nixpkgs/package.nix {}"
./.scratch/nixpkgs-hoard/result/bin/hoard --version
./.scratch/nixpkgs-hoard/result/bin/hoard-gguf --version
```

To recompute a hash after changing the expression, replace the value with
`lib.fakeHash`, re-run the `nix-build` command, and copy the reported `got:`
hash back.
