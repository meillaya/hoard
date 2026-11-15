{
  description = "hoard - verifiable, content-addressed artifacts for large model files";

  # `devenv.nix` is the single source for the development shell. devenv and
  # rust-overlay follow the project nixpkgs, so the package, `devenv shell`,
  # and `nix develop` build the same tools. Full revisions keep lock
  # resolution off the GitHub API.
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/d5cc6833999746cffdfe0598f7bc801a57b887a9";
    devenv = {
      url = "github:cachix/devenv/b904dcb51fe48c30db250038241507f60752f222";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.rust-overlay.follows = "rust-overlay";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay/67f1944c7c99d8dbb57fa3d83043bc0a92cbcc56";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      devenv,
      ...
    }@inputs:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

      mkHoard =
        pkgs:
        pkgs.rustPlatform.buildRustPackage {
          pname = "hoard";
          inherit version;
          src = self;

          # `cargoLock` makes the dependency set offline and reproducible: each
          # crate comes from the lockfile, so no aggregate vendor hash is
          # needed. The default cargo install hook copies every workspace
          # binary (`hoard`, `hoard-gguf`) into $out/bin from the target
          # subdirectory. Integration tests that need `fixtures/real/` skip
          # when it is absent, which is the case inside the Nix sandbox.
          cargoLock.lockFile = ./Cargo.lock;

          cargoBuildFlags = [ "--workspace" ];
          cargoTestFlags = [ "--workspace" ];

          meta = with pkgs.lib; {
            description = "Strict GGUF inspection, manifests, and content hashes for large model files";
            license = licenses.asl20;
            mainProgram = "hoard";
            platforms = platforms.unix;
          };
        };
    in
    {
      packages = forAllSystems (
        pkgs:
        let
          hoard = mkHoard pkgs;
        in
        {
          hoard = hoard;
          default = hoard;
        }
      );

      devShells = forAllSystems (pkgs: {
        default = devenv.lib.mkShell {
          inherit inputs pkgs;
          modules = [
            # Flakes evaluate purely by default, where PWD is not available.
            # Use `--impure` for the real working directory; the fallback lets
            # `nix flake check` evaluate the shell without failing.
            ({ lib, ... }: {
              devenv.root = lib.mkForce (
                let
                  pwd = builtins.getEnv "PWD";
                in
                if pwd != "" then pwd else "/tmp/hoard-devenv"
              );
            })
            ./devenv.nix
          ];
        };
      });

      apps = forAllSystems (
        pkgs:
        let
          hoard = mkHoard pkgs;
        in
        {
          default = {
            type = "app";
            program = "${hoard}/bin/hoard";
          };
          hoard-gguf = {
            type = "app";
            program = "${hoard}/bin/hoard-gguf";
          };
        }
      );

      checks = forAllSystems (pkgs: {
        default = mkHoard pkgs;
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
