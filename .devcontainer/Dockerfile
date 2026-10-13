FROM mcr.microsoft.com/devcontainers/rust:1-bookworm

ARG BUN_VERSION=1.3.3

USER root

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        clang \
        libclang-dev \
        libssl-dev \
        libsqlite3-dev \
        libzstd-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && curl -fsSL https://bun.sh/install | bash -s -- bun-v${BUN_VERSION} \
    && install -m 0755 /root/.bun/bin/bun /usr/local/bin/bun \
    && ln -sf /usr/local/bin/bun /usr/local/bin/bunx \
    && rm -rf /root/.bun

USER vscode

ENV RUST_BACKTRACE=1 \
    CARGO_TERM_COLOR=always
