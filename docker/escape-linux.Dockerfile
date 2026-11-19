# Linux image for the escape test suite (docs/ESCAPE_TESTS.md).
#
# Unlike linux-dev.Dockerfile, this image has no WASI SDK, QEMU, or Chromium, so it builds on
# arm64 (Apple Silicon Docker) as well as amd64. The escape suite needs only a Rust toolchain
# and protoc: it exercises namespaces, seccomp, and cgroup v2, not the VM tier.
# Build and run it with docker/run-escape-tests.sh.
#
# The container is run with --privileged by run-escape-tests.sh so the test suite can create
# namespaces and configure cgroups. No untrusted input is processed.
FROM rust:1.97-bookworm

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
    clang libprotobuf-dev lld protobuf-compiler \
 && rustup component add rustfmt clippy \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src

RUN useradd -m -s /bin/bash sandbox && chown -R sandbox:sandbox /src
USER sandbox
