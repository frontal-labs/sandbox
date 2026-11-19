# Linux image for the escape test suite (docs/ESCAPE_TESTS.md).
#
# Unlike linux-dev.Dockerfile, this image has no WASI SDK, QEMU, or Chromium, so it builds on
# arm64 (Apple Silicon Docker) as well as amd64. The escape suite needs only a Rust toolchain
# and protoc: it exercises namespaces, seccomp, and cgroup v2, not the VM tier.
# Build and run it with docker/run-escape-tests.sh.
FROM rust:1.97-bookworm

RUN apt-get update \
 && apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev clang lld \
 && rm -rf /var/lib/apt/lists/*

RUN rustup component add rustfmt clippy

WORKDIR /src
