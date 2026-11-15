{
  description = "hoard - verifiable, content-addressed artifacts for large model files";

  # nixpkgs is locked to a nixos-unstable revision in flake.lock. Updating it
  # resolves the branch through the GitHub API, which is rate-limited in some
  # sandboxes and CI; if that fails, pin a full commit rev here deliberately
  # and re-run `nix flake lock`.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
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
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            git
            curl
            cacert
            jq
            shellcheck
          ];

          RUST_BACKTRACE = "1";
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
