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
IMAGE="${REGISTRY}/${NAMESPACE}/frontal-sandbox:staging"
COMPOSE_FILE="deploy/compose.staging.yaml"

export RUSTY_V8_MIRROR="https://github.com/denoland/rusty_v8/releases/download"
export RUSTY_V8_MIRROR_FALLBACK="1"
export V8_FROM_SOURCE="0"

echo "deploy-staging: building release binary"
cargo build --release -p cli --bin frontal-sandbox

echo "deploy-staging: building and pushing image"
docker build -f deploy/Dockerfile -t "${IMAGE}" .
docker push "${IMAGE}"

if [[ -f "$COMPOSE_FILE" ]]; then
  echo "deploy-staging: rolling out via $COMPOSE_FILE"
  docker compose -f "$COMPOSE_FILE" up -d --force-recreate
fi

echo "deploy-staging: smoke test"
SANDBOX_BASE_URL="${STAGING_BASE_URL:-http://127.0.0.1:8080}" \
  SANDBOX_API_KEY="${STAGING_API_KEY:-}" \
  ./deploy/scripts/smoke.sh "$SANDBOX_BASE_URL"

echo "deploy-staging: done"