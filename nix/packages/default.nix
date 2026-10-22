# Custom tools the dev shell and the checks use. Each tool gets its own file
# here and is listed in this attribute set.
{ pkgs }:
{
  # `bazel` for the dev shell. It runs bazelisk, which reads .bazelversion and
  # fetches that Bazel release, so the Bazel version is pinned in the repo and
  # not in Nix.
  bazel = pkgs.writeShellApplication {
    name = "bazel";
    runtimeInputs = [ pkgs.bazelisk ];
    text = ''
      exec bazelisk "$@"
    '';
  };
}
