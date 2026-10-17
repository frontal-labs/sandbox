# syntax=docker/dockerfile:1.7
# =============================================================================
# Frontal Sandbox server image
# =============================================================================
# Build context is the repository root. Use docker/compose.yaml, which sets it:
#   docker compose -f docker/compose.yaml build
#
# The image runs `sandbox serve`, the CLI subcommand that starts the HTTP API
# (crates/sandbox-server). Session code runs inside that process, so the
# container is the isolation boundary for the Tier 1 isolate. See docker/README.md.
# =============================================================================

ARG RUST_VERSION=1.95
ARG DEBIAN_SUITE=bookworm

# -----------------------------------------------------------------------------
# Builder
# -----------------------------------------------------------------------------
FROM rust:${RUST_VERSION}-${DEBIAN_SUITE} AS builder

# protoc: crates/sandbox-proto/build.rs needs it at build time. libprotobuf-dev
# supplies the well-known types (google/protobuf/empty.proto) that protoc
# imports. protobuf-compiler alone does not include them on Debian.
# clang/libclang, libssl, libsqlite3, libzstd, pkg-config: the same system
# packages .devcontainer/Dockerfile installs, so the build matches local dev.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        clang \
        libclang-dev \
        libprotobuf-dev \
        libsqlite3-dev \
        libssl-dev \
        libzstd-dev \
        pkg-config \
        protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Copy the whole workspace. Cargo needs every member manifest to resolve the
# workspace, even though only the `cli` package is built. docker/Dockerfile.dockerignore
# trims the context.
COPY . .

# Cache mounts keep the crate registry and build artifacts between builds.
# The binary is copied out inside the same RUN, because /src/target is a cache
# mount and is not kept in the layer.
#
# rusty_v8 (pulled in by deno_core through `cli`) downloads a prebuilt V8
# archive during the build, so this stage needs network access.
# SANDBOX_VERSION is read by cli/src/commands.rs at build time, so `frontal-sandbox
# --version` reports the release. docker.yml passes the tag; a plain build gets
# the placeholder.
ARG SANDBOX_VERSION=0.0.0-dev
ENV SANDBOX_VERSION=${SANDBOX_VERSION}

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked -p cli \
    && install -Dm0755 target/release/frontal-sandbox /out/frontal-sandbox

# -----------------------------------------------------------------------------
# Runtime
# -----------------------------------------------------------------------------
FROM debian:${DEBIAN_SUITE}-slim AS runtime

# curl: used by the HEALTHCHECK. tini: PID 1, so signals reach the server and
# zombie processes are reaped.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        tini \
    && rm -rf /var/lib/apt/lists/*

# Unprivileged system user. API key digests and snapshots live in the state
# directory, so it is owner-only.
RUN groupadd --system --gid 10001 sandbox \
    && useradd --system --uid 10001 --gid sandbox \
        --home-dir /var/lib/sandbox --shell /usr/sbin/nologin sandbox \
    && install -d -o sandbox -g sandbox -m 0700 /var/lib/sandbox

COPY --from=builder /out/frontal-sandbox /usr/local/bin/frontal-sandbox

USER sandbox:sandbox
WORKDIR /var/lib/sandbox

ENV RUST_BACKTRACE=1

# The server binds 0.0.0.0:8080 inside the container. compose.yaml decides what
# the host exposes, and by default that is loopback only.
EXPOSE 8080

# /v1/openapi.json is the only unauthenticated route, so it is the liveness
# probe. Every other /v1 route needs an API key.
HEALTHCHECK --interval=15s --timeout=3s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/v1/openapi.json > /dev/null || exit 1

# Durable mode: keys, snapshots, and sleeping sessions survive a restart
# (ADR 0008). The first start prints one API key to the container log.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/frontal-sandbox"]
CMD ["serve", "--addr", "0.0.0.0:8080", "--data-dir", "/var/lib/sandbox"]
