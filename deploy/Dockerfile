# Production-hardened container image for Frontal Sandbox.
#
# Extends docker/Dockerfile with TLS support and production defaults.
#
# Build (from the repository root):
#   docker build -f deploy/Dockerfile -t frontal-sandbox:prod .
#
# Run with deploy/compose.prod.yaml:
#   docker compose -f docker/compose.yaml -f deploy/compose.prod.yaml up -d

ARG RUST_VERSION=1.95
ARG DEBIAN_SUITE=bookworm

# -----------------------------------------------------------------------------
# Builder
# -----------------------------------------------------------------------------
FROM rust:${RUST_VERSION}-${DEBIAN_SUITE} AS builder

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
COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked -p cli \
    && install -Dm0755 target/release/sandbox /out/sandbox

# -----------------------------------------------------------------------------
# Runtime
# -----------------------------------------------------------------------------
FROM debian:${DEBIAN_SUITE}-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        tini \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --system --gid 10001 sandbox \
    && useradd --system --uid 10001 --gid sandbox \
        --home-dir /var/lib/sandbox --shell /usr/sbin/nologin sandbox \
    && install -d -o sandbox -g sandbox -m 0700 /var/lib/sandbox \
    && install -d -o sandbox -g sandbox -m 0755 /etc/sandbox \
    && install -d -o sandbox -g sandbox -m 0755 /var/run/sandbox

COPY --from=builder /out/sandbox /usr/local/bin/sandbox

USER sandbox:sandbox
WORKDIR /var/lib/sandbox

ENV RUST_BACKTRACE=1

EXPOSE 8080
EXPOSE 9090

HEALTHCHECK --interval=15s --timeout=3s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/v1/openapi.json > /dev/null || exit 1

# Production default: durable mode with config file and data dir.
# Operators mount TLS certs into /etc/sandbox/tls and pass --config.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/sandbox"]
CMD ["serve", "--config", "/etc/sandbox/config.toml", "--addr", "0.0.0.0:8080", "--data-dir", "/var/lib/sandbox"]
