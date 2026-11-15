{
  description = "hoard model artifact consumer example";

  inputs = {
    hoard.url = "github:meillaya/hoard";
    nixpkgs.follows = "hoard/nixpkgs";
  };

  outputs =
    {
      self,
      hoard,
      nixpkgs,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f system nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (
        system: pkgs: {
          default = hoard.lib.fetchArtifactSet {
            inherit pkgs;
            hoard = hoard.packages.${system}.default;
            manifest = ../models/stories15M-q8_0.json;
          };
        }
      );
    };
}
