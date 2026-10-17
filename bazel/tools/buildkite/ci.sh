#!/usr/bin/env bash
# =============================================================================
# Runs one Bazel phase for a Buildkite step.
#
#   bazel/tools/buildkite/ci.sh build
#   bazel/tools/buildkite/ci.sh test
#
# Set BUILDBARN_CACHE_URL on the agent (for example grpc://cache.internal:8980)
# to use the remote cache. Without it, the phase still runs, but without a cache.
#
# Targets are the aliases in the root BUILD.bazel, so this file changes only when
# a phase's scope does.
#
# The script runs under macOS's default /bin/bash (3.2). Keep it free of bash-4
# features. The ${arr[@]+...} form avoids an unbound-variable error on an empty
# array under `set -u`.
# =============================================================================
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../../.."

phase="${1:-}"
case "${phase}" in
  build)
    bazel_command=build
    targets=(//:build_all //:build_rust //:build_sdks)
    ;;
  test)
    bazel_command=test
    targets=(//:test_rust //:test_ts)
    ;;
  *)
    echo "usage: $0 build|test" >&2
    exit 2
    ;;
esac

cache_flags=()
if [[ -n "${BUILDBARN_CACHE_URL:-}" ]]; then
  cache_flags=(--config=remote-cache "--remote_cache=${BUILDBARN_CACHE_URL}")
  echo "remote cache: ${BUILDBARN_CACHE_URL}"
else
  echo "BUILDBARN_CACHE_URL is not set: running without a remote cache" >&2
fi

exec bazel "${bazel_command}" --config=ci ${cache_flags[@]+"${cache_flags[@]}"} "${targets[@]}"
