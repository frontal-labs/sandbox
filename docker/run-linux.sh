#!/bin/sh
# Runs a command inside the Linux dev image against this checkout.
#
#   docker/run-linux.sh cargo test -p sandbox-worker
#
# The checkout is mounted at /src. Cargo's registry and the build output live in
# named volumes, so macOS and Linux build artifacts never mix.
#
# The container needs a delegated cgroup for worker limits (SANDBOX_CGROUP_ROOT).
# The entrypoint moves every process into /sys/fs/cgroup/host, which leaves the
# root empty, enables the controllers at the root, and creates /sys/fs/cgroup/workers
# with the controllers enabled for its children. That is what a systemd unit with
# Delegate=yes gives a real host.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
IMAGE=frontal-sandbox-linux-dev

docker build -q -t "$IMAGE" -f "$ROOT/docker/linux-dev.Dockerfile" "$ROOT/docker" >/dev/null

exec docker run --rm --privileged \
  -v "$ROOT:/src" \
  -v frontal-sandbox-cargo-registry:/usr/local/cargo/registry \
  -v frontal-sandbox-linux-target:/cache/target \
  -e CARGO_TARGET_DIR=/cache/target \
  -e CARGO_TERM_COLOR=never \
  -e SANDBOX_CGROUP_ROOT=/sys/fs/cgroup/workers \
  -w /src \
  "$IMAGE" sh -c '
    set -e
    mkdir -p /sys/fs/cgroup/host /sys/fs/cgroup/workers
    echo $$ > /sys/fs/cgroup/host/cgroup.procs
    echo "+cpu +memory +pids" > /sys/fs/cgroup/cgroup.subtree_control
    echo "+cpu +memory +pids" > /sys/fs/cgroup/workers/cgroup.subtree_control
    exec "$@"
  ' sh "$@"
