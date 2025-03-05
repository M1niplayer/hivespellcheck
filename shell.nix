{
  pkgs ? (import <nixpkgs> { }),
  unstable ? (import <unstable> { }),
}:
pkgs.mkShellNoCC {
  buildInputs = with pkgs; [
    rustc
    cargo
    rustfmt
    gcc # serde needs cc linker

    rust-analyzer
    clippy
    cargo-make
    sqlx-cli
  ];

  shellHook = ''
    export RUST_SRC_PATH="${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}"

    mkdir -p .vscode
    cat > .vscode/settings.json <<EOF
    // Auto-generated by shell.nix
    {
      "rust-analyzer.cargo.sysrootSrc": "$RUST_SRC_PATH",
      "rust-analyzer.check.command": "clippy"
    }
    EOF
  '';
}
