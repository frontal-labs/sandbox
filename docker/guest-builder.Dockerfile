# Builds the Tier 2 guest image (kernel, agent, ffmpeg, Chromium) for the architecture the
# container runs on. It is the lean counterpart of linux-dev.Dockerfile: no WASI SDK, so it
# builds on arm64 hosts (Apple Silicon), where the guest then runs under HVF.
# Use docker/build-guest-image.sh, which builds this image and runs build-image.sh in it.
FROM rust:1.97-bookworm

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
    protobuf-compiler libprotobuf-dev \
    "linux-image-$(dpkg --print-architecture)" busybox-static cpio kmod xz-utils \
    ffmpeg e2fsprogs chromium \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
