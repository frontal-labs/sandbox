#!/bin/sh
# Starts an S3-compatible service in Docker (adobe/s3mock), creates a bucket, and runs the
# object-store tests against it.
#
#   docker/object-store-test.sh
#
# The tests use only the S3 API, so the same run works against MinIO or AWS S3 when those are
# available. adobe/s3mock is used here because it is public on Docker Hub; MinIO's images were
# not pullable from the registries this was run against.
set -eu

NAME=sandbox-cas-s3
BUCKET=sandbox-cas-test
ROOT=$(cd "$(dirname "$0")/.." && pwd)

docker rm -f "$NAME" >/dev/null 2>&1 || true
docker run -d --name "$NAME" -p 127.0.0.1:9090:9090 \
  adobe/s3mock:latest >/dev/null
trap 'docker rm -f "$NAME" >/dev/null 2>&1 || true' EXIT

i=0
until curl -fsS -o /dev/null http://127.0.0.1:9090/ 2>/dev/null; do
  i=$((i + 1)); [ "$i" -lt 60 ] || { echo "the S3 service did not start" >&2; exit 1; }
  sleep 1
done

# Create the bucket with the S3 API: the image does not create buckets from its environment.
curl -fsS -X PUT "http://127.0.0.1:9090/$BUCKET" >/dev/null

cd "$ROOT"
# s3mock does not check credentials, but the S3 client needs some to sign requests.
SANDBOX_S3_ENDPOINT=http://127.0.0.1:9090 \
SANDBOX_S3_BUCKET="$BUCKET" \
AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test AWS_REGION=us-east-1 \
  cargo test -p sandbox-cas --test s3 -- --ignored --nocapture
