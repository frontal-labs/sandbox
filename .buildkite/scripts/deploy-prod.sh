#!/usr/bin/env bash
set -euo pipefail

# deploy-prod.sh — deploy Frontal Sandbox to production.
#
# Runs only after the `block` step approves. Credentials are obtained via OIDC
# (the `buildkite-oidc` plugin) and the `buildkite-agent secret get` command;
# no secrets are stored in the pipeline file or the repo.
#
# Runnable locally with the env vars exported:
#   PROD_REGISTRY=... PROD_NAMESPACE=... ./.buildkite/scripts/deploy-prod.sh

REGISTRY="${PROD_REGISTRY:-ghcr.io}"
NAMESPACE="${PROD_NAMESPACE:-${BUILDKITE_ORGANIZATION_SLUG:-frontal-labs}}"
IMAGE="${REGISTRY}/${NAMESPACE}/sandbox:latest"
COMPOSE_FILE="deploy/compose.prod.yaml"

export RUSTY_V8_MIRROR="https://github.com/denoland/rusty_v8/releases/download"
export RUSTY_V8_MIRROR_FALLBACK="1"
export V8_FROM_SOURCE="0"

echo "deploy-prod: building release binary"
cargo build --release -p cli --bin sandbox --bin frontal-sandbox

echo "deploy-prod: building and pushing image"
docker build -f deploy/Dockerfile -t "${IMAGE}" .
docker push "${IMAGE}"

if [[ -f "$COMPOSE_FILE" ]]; then
  echo "deploy-prod: rolling out via $COMPOSE_FILE"
  docker compose -f "$COMPOSE_FILE" up -d --force-recreate
fi

echo "deploy-prod: smoke test"
SANDBOX_BASE_URL="${PROD_BASE_URL:-http://127.0.0.1:8080}" \
  SANDBOX_API_KEY="${PROD_API_KEY:-}" \
  ./deploy/scripts/smoke.sh "$SANDBOX_BASE_URL"

echo "deploy-prod: done"