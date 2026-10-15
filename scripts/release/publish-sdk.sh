#!/usr/bin/env bash
# Packs one SDK and pushes it to GHCR as an OCI artifact.
#
#   scripts/release/publish-sdk.sh <ts|py|rs> [--dry-run]
#
# The version is read from sdks/typescript/package.json, since changesets keeps
# all three SDKs on one version. Needs oras and, unless --dry-run, a logged-in
# session (`oras login ghcr.io`). The push is skipped when the version already
# exists, so a rerun is safe.
set -euo pipefail

sdk="${1:?usage: publish-sdk.sh <ts|py|rs> [--dry-run]}"
dry_run=false
[[ "${2:-}" == "--dry-run" ]] && dry_run=true

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$repo_root"

# A real publish must come from a clean tree, so the artifacts match a commit.
if ! $dry_run && [[ -n "$(git status --porcelain)" ]]; then
  echo "refusing to publish from a tree with uncommitted changes" >&2
  exit 1
fi

version="$(node -p "require('./sdks/typescript/package.json').version")"
image="ghcr.io/frontal-labs/sandbox-sdk-${sdk}"
out="$repo_root/release/${sdk}"
rm -rf "$out"
mkdir -p "$out"

case "$sdk" in
  ts)
    (cd sdks/typescript && npm run build >/dev/null && npm pack --pack-destination "$out" >/dev/null)
    artifacts=("$out/sandbox-sdk-${version}.tgz:application/vnd.npm.package.tar+gzip")
    ;;
  py)
    python3 -m build --outdir "$out" sdks/python >/dev/null
    artifacts=(
      "$out/sandbox_sdk-${version}-py3-none-any.whl:application/vnd.python.wheel"
      "$out/sandbox_sdk-${version}.tar.gz:application/gzip"
    )
    ;;
  rs)
    # The crate depends on sandbox-types, which is not on crates.io, so a
    # `cargo package` tarball cannot be built. This is a source snapshot of the
    # workspace at HEAD: committed files only, so untracked local files such as
    # .env never reach the registry. Build it with `cargo build -p sandbox-sdk`.
    prefix="sandbox-sdk-rs-${version}"
    git archive --format=tar --prefix="$prefix/" HEAD | gzip -n > "$out/${prefix}.tar.gz"
    artifacts=("$out/${prefix}.tar.gz:application/gzip")
    ;;
  *)
    echo "unknown SDK: $sdk" >&2
    exit 2
    ;;
esac

for artifact in "${artifacts[@]}"; do
  file="${artifact%%:*}"
  [[ -s "$file" ]] || { echo "missing or empty artifact: $file" >&2; exit 1; }
done

if $dry_run; then
  echo "dry run: would push ${image}:${version}"
  for artifact in "${artifacts[@]}"; do echo "  ${artifact%%:*}"; done
  exit 0
fi

if oras manifest fetch "${image}:${version}" >/dev/null 2>&1; then
  echo "${image}:${version} already exists; nothing to do"
  exit 0
fi

# oras takes "file:media-type" pairs. It rejects absolute paths, so push from
# the output directory with bare names. Each file keeps its name as the title.
push_args=()
for artifact in "${artifacts[@]}"; do
  file="$(basename "${artifact%%:*}")"
  media="${artifact#*:}"
  push_args+=("${file}:${media}")
done
(cd "$out" && oras push "${image}:${version}" "${push_args[@]}")
oras tag "${image}:${version}" latest
echo "pushed ${image}:${version} and tagged latest"
