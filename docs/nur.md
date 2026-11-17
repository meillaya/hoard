# NUR packages

The NUR repository for hoard lives at
[meillaya/nur-packages](https://github.com/meillaya/nur-packages). It packages
`hoard-artifact`, which installs the `hoard` CLI and the `hoard-gguf` inspector.
The attribute is `hoard-artifact` because the `hoard` attribute is used by an
unrelated nixpkgs package.

## Use

Build the package directly from the repository:

```sh
nix build github:meillaya/nur-packages#hoard-artifact
./result/bin/hoard --version
./result/bin/hoard-gguf --version
```

Or add the repository as a flake input:

```nix
{
  inputs.nur-packages.url = "github:meillaya/nur-packages";
}
```

```nix
nur-packages.packages.${system}.hoard-artifact
```

The repository is also submitted to [NUR](https://github.com/nix-community/NUR)
as the `meillaya` repository. After registration, the overlay path is:

```nix
nur.legacyPackages.${system}.repos.meillaya.hoard-artifact
```

## Maintenance

The package expression is in
[`pkgs/hoard-artifact/default.nix`](https://github.com/meillaya/nur-packages/blob/main/pkgs/hoard-artifact/default.nix).
It pins the `v0.1.0` release tag and both digests. Update the tag, `src.hash`,
and `cargoHash` when a new hoard release is published.

The NUR packaging files are MIT licensed. The packaged upstream software is
Apache-2.0; see [the licence](../LICENCE).
