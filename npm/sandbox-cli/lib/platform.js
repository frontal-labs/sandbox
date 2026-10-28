// The prebuilt targets. Each one matches a matrix entry in .github/workflows/release-binaries.yml,
// which publishes it as @frontal-labs/sandbox-cli-<triple>.
export const TARGETS = [
  { triple: "aarch64-apple-darwin", os: "darwin", cpu: "arm64", exe: "frontal-sandbox" },
  { triple: "x86_64-apple-darwin", os: "darwin", cpu: "x64", exe: "frontal-sandbox" },
  { triple: "x86_64-unknown-linux-gnu", os: "linux", cpu: "x64", exe: "frontal-sandbox" },
  { triple: "aarch64-unknown-linux-gnu", os: "linux", cpu: "arm64", exe: "frontal-sandbox" },
  { triple: "x86_64-pc-windows-msvc", os: "win32", cpu: "x64", exe: "frontal-sandbox.exe" },
];

export const PACKAGE_PREFIX = "@frontal-labs/sandbox-cli-";

// Returns the target for a process.platform / process.arch pair, or undefined if none is prebuilt.
export function findTarget(platform, arch) {
  return TARGETS.find((target) => target.os === platform && target.cpu === arch);
}

export function packageName(target) {
  return `${PACKAGE_PREFIX}${target.triple}`;
}
