{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    flake-utils.url = "github:numtide/flake-utils";

    crane.url = "github:ipetkov/crane";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs = {
        nixpkgs.follows = "nixpkgs";
      };
    };
  };

  outputs = { nixpkgs, crane, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };


        rustToolchain = (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml).override {
          targets = [ "x86_64-unknown-linux-musl" ];
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;
        templateFilter = path: _type: builtins.match ".*templates/.*html$" path != null;
        templateOrCargo = path: type: (templateFilter path type) || (craneLib.filterCargoSources path type);
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = templateOrCargo;
          name = "source";
        };

        # Common arguments can be set here to avoid repeating them later
        commonArgs = {
          inherit src;
          strictDeps = true;
        };

        # Build *just* the cargo dependencies, so we can reuse
        # all of that work (e.g. via cachix) when running in CI
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Build the actual crate itself, reusing the dependency
        # artifacts from above.
        bin = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
        });

        dockerImage = pkgs.dockerTools.buildLayeredImage {
          name = "sfsb";
          tag = "latest";

          contents = [ bin ];
          config = {
            Cmd = [ "${bin}/bin/sfsb" ];
            ExposedPorts = { "3779/tcp" = {}; };
          };
        };
      in
      {
        packages = {
          default = bin;
          inherit bin dockerImage;
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ bin ];
        };
      });
}
