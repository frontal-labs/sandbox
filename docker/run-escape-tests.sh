#!/bin/sh
# Runs the escape test suite on Linux, inside a privileged container (docs/ESCAPE_TESTS.md).
#
#   docker/run-escape-tests.sh                 # the whole suite
#   docker/run-escape-tests.sh -- --ignored    # also the tests that record a KNOWN GAP
#   ESCAPE_NAME_SUFFIX=mytrack docker/run-escape-tests.sh   # unique image and volume names
#
# The container needs the same delegated cgroup as docker/run-linux.sh, so the worker's cgroup
# limits can be set. It is privileged, because creating namespaces and cgroups needs CAP_SYS_ADMIN.
# The probes run as root inside the container, which is the weakest setting for the filter
# (a non-root process cannot do some of what the probes try). So a probe the filter blocks as
# root is also blocked for an unprivileged user.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SUFFIX=${ESCAPE_NAME_SUFFIX:+-$ESCAPE_NAME_SUFFIX}
IMAGE=frontal-sandbox-escape$SUFFIX

docker build -q -t "$IMAGE" -f "$ROOT/docker/escape-linux.Dockerfile" "$ROOT/docker" >/dev/null

# Arguments after `--` go to every test binary.
extra=""
if [ "${1:-}" = "--" ]; then
  shift
  extra="$*"
fi

exec docker run --rm --privileged --user 0:0 \
  --name "frontal-sandbox-escape-run$SUFFIX-$$" \
  -v "$ROOT:/src" \
  -v "frontal-sandbox-escape-cargo$SUFFIX:/usr/local/cargo/registry" \
  -v "frontal-sandbox-escape-target$SUFFIX:/cache/target" \
  -e CARGO_TARGET_DIR=/cache/target \
  -e CARGO_TERM_COLOR=never \
  -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" \
  -e SANDBOX_CGROUP_ROOT=/sys/fs/cgroup/workers \
  -e ESCAPE_EXTRA_ARGS="$extra" \
  -w /src \
  "$IMAGE" sh -c '
    set -e
    mkdir -p /sys/fs/cgroup/host /sys/fs/cgroup/workers
    echo $$ > /sys/fs/cgroup/host/cgroup.procs
    echo "+cpu +memory +pids" > /sys/fs/cgroup/cgroup.subtree_control
    echo "+cpu +memory +pids" > /sys/fs/cgroup/workers/cgroup.subtree_control
    status=0
    # The Linux-tier suite (namespaces, seccomp, cgroups), then the guest-level suites that
    # also run on macOS, then the contained-script checks that need the delegated cgroup.
    cargo test -p sandbox-worker --test escape_linux -- $ESCAPE_EXTRA_ARGS || status=1
    cargo test -p sandbox-wasm --test escape -- $ESCAPE_EXTRA_ARGS || status=1
    cargo test -p sandbox-shell --test escape -- $ESCAPE_EXTRA_ARGS || status=1
    cargo test -p sandbox-runtime --test escape -- $ESCAPE_EXTRA_ARGS || status=1
    cargo test -p cli --features contained-tests --test contained_escape -- $ESCAPE_EXTRA_ARGS || status=1
    exit $status
  ' sh
