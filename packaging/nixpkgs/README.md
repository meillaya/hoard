# Nixpkgs packaging

`package.nix` is the package expression submitted as
[NixOS/nixpkgs#588800](https://github.com/NixOS/nixpkgs/pull/588800). It is also
mirrored in the pull request at
`pkgs/by-name/ho/hoard-artifact/package.nix`.

## Naming

The attribute and by-name directory are `hoard-artifact` because `hoard` already
exists in nixpkgs (Hyde46/hoard, a different project). `pname` stays `hoard` for
the upstream project and the installed binary.

## Package contents

- Builds the whole Cargo workspace with `buildRustPackage`.
- Installs `hoard` and `hoard-gguf`.
- Uses `pkg-config` and `aws-lc` because the dependency tree enables
  `aws-lc-rs` through `rustls` and nixpkgs exports `AWS_LC_SYS_USE_SYSTEM=1`.
- Exports the nixpkgs CA bundle before checks because the Hub client tests build
  a `reqwest` client that reads the system root store.
- Pins tag `v0.1.0` and carries the `meillaya` maintainer entry.

## Validation

The expression was built against nixpkgs
`d5cc6833999746cffdfe0598f7bc801a57b887a9`:

```sh
nix-build -E "with import <nixpkgs> {}; callPackage ./packaging/nixpkgs/package.nix {}"
```

`nixpkgs-review pr 588800` reports one package built on `x86_64-linux`
(`hoard-artifact`). The nixpkgs eval, build, lint, and check jobs pass for
`x86_64-linux`, `aarch64-linux`, and `aarch64-darwin`.
