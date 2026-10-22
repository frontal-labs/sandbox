{
  description = "Frontal Sandbox development environment and repository checks";

  # Entry point only. Each concern lives in nix/: overlays.nix, packages/,
  # devshell.nix and checks.nix. Every input version is pinned by flake.lock.
  # Bazel owns the build (see MODULE.bazel and docs/build-system.md) and is
  # fetched by bazelisk from .bazelversion, so it is not pinned here.
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = import ./nix/overlays.nix { inherit rust-overlay; };
        };
        customPackages = import ./nix/packages { inherit pkgs; };
      in {
        packages = customPackages;
        devShells.default = import ./nix/devshell.nix {
          inherit pkgs;
          bazel = customPackages.bazel;
        };
        checks = import ./nix/checks.nix { inherit pkgs; };
        formatter = pkgs.nixfmt-rfc-style;
      });
}
