// The prebuilt targets. Each one matches a matrix entry in .github/workflows/release-binaries.yml,
// which publishes it as @frontal-labs/sandbox-cli-<triple>.
//
// The canonical binary is `sandbox`; `frontal-sandbox` is a legacy alias built from the
// same source. Both are staged into every platform package, so a user can invoke either name.
export const TARGETS = [
  { triple: "aarch64-apple-darwin", os: "darwin", cpu: "arm64", exe: "sandbox", alias: "frontal-sandbox" },
  { triple: "x86_64-apple-darwin", os: "darwin", cpu: "x64", exe: "sandbox", alias: "frontal-sandbox" },
  { triple: "x86_64-unknown-linux-gnu", os: "linux", cpu: "x64", exe: "sandbox", alias: "frontal-sandbox" },
  { triple: "aarch64-unknown-linux-gnu", os: "linux", cpu: "arm64", exe: "sandbox", alias: "frontal-sandbox" },
  { triple: "x86_64-pc-windows-msvc", os: "win32", cpu: "x64", exe: "sandbox.exe", alias: "frontal-sandbox.exe" },
];

export const PACKAGE_PREFIX = "@frontal-labs/sandbox-cli-";

// Returns the target for a process.platform / process.arch pair, or undefined if none is prebuilt.
export function findTarget(platform, arch) {
  return TARGETS.find((target) => target.os === platform && target.cpu === arch);
}

export function packageName(target) {
  return `${PACKAGE_PREFIX}${target.triple}`;
}

// The binary name to spawn, chosen from the script that invoked this wrapper. The
// canonical `sandbox` is preferred; `frontal-sandbox` is the legacy alias, so the
// wrapper spawns the alias when invoked as the alias and the canonical name otherwise.
export function exeFor(argv0) {
  const name = argv0 ? argv0.split(/[\\/]/).pop() : "";
  if (name === "frontal-sandbox" || name === "frontal-sandbox.exe") {
    return name;
  }
  if (name === "sandbox" || name === "sandbox.exe") {
    return name;
  }
  return "sandbox";
}