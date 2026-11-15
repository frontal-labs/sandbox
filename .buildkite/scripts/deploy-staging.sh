#!/usr/bin/env bash
set -euo pipefail

# deploy-staging.sh — deploy Frontal Sandbox to the staging environment.
#
# Triggered on pushes to the default branch. Builds the release binary, pushes
# the Docker image to the staging registry, and rolls out via the staging
# compose manifest.
#
# Credentials come from the Buildkite agent's secret store or OIDC; nothing is
# committed to the repo. Runnable locally with the env vars exported:
#   STAGING_REGISTRY=... STAGING_NAMESPACE=... ./.buildkite/scripts/deploy-staging.sh

REGISTRY="${STAGING_REGISTRY:-ghcr.io}"
NAMESPACE="${STAGING_NAMESPACE:-${BUILDKITE_ORGANIZATION_SLUG:-frontal-labs}}"
IMAGE="${REGISTRY}/${NAMESPACE}/sandbox:staging"
COMPOSE_FILE="infrastructure/docker/compose.staging.yaml"

export RUSTY_V8_MIRROR="https://github.com/denoland/rusty_v8/releases/download"
export RUSTY_V8_MIRROR_FALLBACK="1"
export V8_FROM_SOURCE="0"

echo "deploy-staging: building release binary"
cargo build --release -p cli --bin sandbox --bin frontal-sandbox

echo "deploy-staging: building and pushing image"
docker build -f docker/Dockerfile -t "${IMAGE}" .
docker push "${IMAGE}"

if [[ -f "$COMPOSE_FILE" ]]; then
  echo "deploy-staging: rolling out via $COMPOSE_FILE"
  # Pin the rollout to the image we just pushed so Compose doesn't fall back
  # to the local ${SANDBOX_IMAGE:-frontal-sandbox:staging} tag. Compose resolves
  # paths in later files relative to the first file's directory (docker/), so
  # the base file goes first and the overlay's build context resolves to the
  # repository root.
  SANDBOX_IMAGE="${IMAGE}" docker compose -f docker/compose.yaml -f "$COMPOSE_FILE" up -d --force-recreate
fi

echo "deploy-staging: smoke test"
SANDBOX_BASE_URL="${STAGING_BASE_URL:-http://127.0.0.1:8080}" \
  SANDBOX_API_KEY="${STAGING_API_KEY:-}" \
  bash ./infrastructure/scripts/smoke.sh "$SANDBOX_BASE_URL"

echo "deploy-staging: done"