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

          # reqwest and hf-xet enable aws-lc-rs through rustls. nixpkgs sets
          # AWS_LC_SYS_USE_SYSTEM=1 for Rust builds, so provide the library
          # instead of letting aws-lc-sys try to find one.
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.aws-lc ];
          env = {
            SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
            SSL_CERT_DIR = "${pkgs.cacert}/etc/ssl/certs";
          };

          cargoBuildFlags = [ "--workspace" ];
          cargoTestFlags = [ "--workspace" ];

          meta = with pkgs.lib; {
            description = "Strict GGUF inspection, manifests, and content hashes for large model files";
            license = licenses.asl20;
            mainProgram = "hoard";
            platforms = platforms.unix;
          };
        };
      fetchArtifact =
        {
          pkgs,
          hoard,
          manifest,
          path,
        }:
        let
          data = builtins.fromJSON (builtins.readFile manifest);
          matches = builtins.filter (file: file.path == path) data.files;
          entry =
            if matches == [ ] then
              throw "fetchArtifact: ${path} is not in ${toString manifest}"
            else
              builtins.head matches;
          name = "hoard-${builtins.substring 0 12 entry.sha256}-${baseNameOf path}";
          outputHash = builtins.convertHash {
            hash = "sha256:${entry.sha256}";
            toHashFormat = "sri";
          };
        in
        pkgs.stdenvNoCC.mkDerivation {
          inherit name outputHash;
          outputHashAlgo = "sha256";
          outputHashMode = "flat";
          impureEnvVars = [ "HF_TOKEN" ];
          nativeBuildInputs = [ pkgs.cacert ];
          buildCommand = ''
            export HOME="$TMPDIR"
            export XDG_CACHE_HOME="$TMPDIR/.cache"
            export HOARD_STORE="$TMPDIR/hoard-store"
            export SSL_CERT_FILE="${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
            mkdir -p "$(dirname "$out")"
            ${hoard}/bin/hoard fetch --manifest ${pkgs.lib.escapeShellArg (toString manifest)} --file ${pkgs.lib.escapeShellArg path} --out "$out"
          '';
        };

      fetchArtifactSet =
        {
          pkgs,
          hoard,
          manifest,
        }:
        let
          data = builtins.fromJSON (builtins.readFile manifest);
          items = map (entry: {
            inherit (entry) path;
            artifact = fetchArtifact {
              inherit pkgs hoard manifest;
              path = entry.path;
            };
          }) data.files;
          farmName = "hoard-set-${builtins.substring 0 12 (builtins.head data.files).sha256}";
        in
        pkgs.runCommand farmName { } ''
          mkdir -p "$out"
          ${pkgs.lib.concatMapStringsSep "
" (item: ''
            mkdir -p "$out/$(dirname ${pkgs.lib.escapeShellArg item.path})"
            ln -s ${item.artifact} "$out/${item.path}"
          '') items}
        '';

    in
    {
      lib = {
        inherit fetchArtifact fetchArtifactSet;
      };

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
