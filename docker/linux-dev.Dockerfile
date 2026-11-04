# Linux build and test environment for the parts of Frontal Sandbox that need
# Linux: worker isolation (PLAN.md §4.13), cgroups, namespaces, and seccomp, and
# the Bazel Linux targets (bazel/platforms), and the emulated guest VM
# (docker/run-emulated-vm.sh: QEMU's software emulator, since no KVM is available).
# Build and run it with docker/run-linux.sh. The container runs privileged, because namespace and
# cgroup tests need CAP_SYS_ADMIN.
FROM rust:1.95-bookworm

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
    protobuf-compiler libprotobuf-dev \
    python3 unzip zip curl ca-certificates \
    clang lld wasi-libc libclang-rt-14-dev-wasm32 \
    qemu-system-x86 linux-image-amd64 busybox-static cpio kmod xz-utils \
    ffmpeg e2fsprogs chromium \
 && rm -rf /var/lib/apt/lists/*

RUN rustup component add rustfmt clippy

# The WASI SDK builds CPython for wasm32-wasip1 (tools/wasm/python). Pinned by version and
# checksum, for x86_64 hosts only: the arm64 release is not pinned yet, so other
# architectures fail here rather than fetching an unverified archive.
ARG WASI_SDK_VERSION=25
ARG WASI_SDK_SHA256=52640dde13599bf127a95499e61d6d640256119456d1af8897ab6725bcf3d89c
RUN set -eu; \
    [ "$(dpkg --print-architecture)" = amd64 ] || { echo "WASI SDK is pinned for amd64 only" >&2; exit 1; }; \
    curl -fsSL --proto '=https' --tlsv1.2 -o /tmp/wasi-sdk.tgz \
      "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-${WASI_SDK_VERSION}/wasi-sdk-${WASI_SDK_VERSION}.0-x86_64-linux.tar.gz"; \
    echo "${WASI_SDK_SHA256}  /tmp/wasi-sdk.tgz" | sha256sum -c -; \
    tar -xzf /tmp/wasi-sdk.tgz -C /opt; \
    mv "/opt/wasi-sdk-${WASI_SDK_VERSION}.0-x86_64-linux" /opt/wasi-sdk; \
    rm /tmp/wasi-sdk.tgz
ENV WASI_SDK_PATH=/opt/wasi-sdk

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
