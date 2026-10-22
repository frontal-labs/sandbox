# nixpkgs overlays, applied once where flake.nix imports nixpkgs.
#
# rust-overlay supplies rust-bin, which reads the toolchain from
# rust-toolchain.toml. Add nixpkgs patches or package pins here, not in the
# other files, so every package set in the flake sees the same changes.
{ rust-overlay }:
[
  rust-overlay.overlays.default
]
