{
  pkgs,
  config,
  ...
}:
{
  languages.rust = {
    enable = true;
    toolchainFile = ./rust-toolchain.toml;
  };

  packages = with pkgs; [
    cacert
    cmake
    curl
    git
    jq
    nix
    perl
    pkg-config
    (python3.withPackages (ps: [ ps.numpy ps.pyyaml ]))
    shellcheck
  ];

  env = {
    CARGO_HOME = "${config.devenv.root}/.cargo-home";
    CARGO_TERM_COLOR = "always";
    RUST_BACKTRACE = "1";
    SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
  };

  scripts = {
    test.exec = "cargo test --workspace --all-features";
    check.exec = "cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features";
    fixtures.exec = "bash scripts/fetch-fixtures.sh";
    xet-live.exec = "cargo test -p hoard-hf --test live_xet -- --ignored";
    roundtrip.exec = "bash scripts/check-nix-roundtrip.sh";
    bench.exec = "bash scripts/benchmark-fetch.sh";
  };

  enterShell = ''
    printf 'rustc  %s\n' "$(rustc --version)"
    printf 'cargo  %s\n' "$(cargo --version)"
    printf 'nix    %s\n' "$(nix --version)"
    echo
    echo 'check:  devenv test'
    echo 'tests:  cargo test --workspace --all-features'
  '';

  enterTest = ''
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-features
  '';
}
