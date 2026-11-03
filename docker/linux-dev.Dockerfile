# Linux build and test environment for the parts of Frontal Sandbox that need
# Linux: worker isolation (PLAN.md §4.13), cgroups, namespaces, and seccomp, and
# the Bazel Linux targets (bazel/platforms). Build and run it with
# docker/run-linux.sh. The container runs privileged, because namespace and
# cgroup tests need CAP_SYS_ADMIN.
FROM rust:1.95-bookworm

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
    protobuf-compiler libprotobuf-dev \
    python3 unzip zip curl ca-certificates \
    clang lld wasi-libc libclang-rt-14-dev-wasm32 \
 && rm -rf /var/lib/apt/lists/*

RUN rustup component add rustfmt clippy

# The image runs as an unprivileged user by default. docker/run-linux.sh starts the
# container as root explicitly, because the isolation tests need root.
RUN useradd -m -u 1000 dev

# Bazelisk reads .bazelversion and fetches the pinned Bazel. Pinned by version, and
# picked by the container's architecture.
ARG BAZELISK_VERSION=v1.25.0
RUN arch="$(dpkg --print-architecture)" \
 && curl -fsSL --proto '=https' --tlsv1.2 --proto-redir '=https' -o /usr/local/bin/bazel \
    "https://github.com/bazelbuild/bazelisk/releases/download/${BAZELISK_VERSION}/bazelisk-linux-${arch}" \
 && chmod +x /usr/local/bin/bazel

USER dev

WORKDIR /src
