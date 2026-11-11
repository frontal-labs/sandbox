#!/bin/sh
# Starts Postgres and an S3-compatible service in Docker, then runs the clustered tests that
# need both (the state store, and the object-store blobs of a clustered server).
#
#   docker/clustered-test.sh
set -eu

PG=sandbox-clustered-pg
S3=sandbox-clustered-s3
ROOT=$(cd "$(dirname "$0")/.." && pwd)
PGPASS=sandbox-test

docker rm -f "$PG" "$S3" >/dev/null 2>&1 || true
docker run -d --name "$PG" -p 127.0.0.1:55432:5432 -e POSTGRES_PASSWORD="$PGPASS" postgres:16 >/dev/null
docker run -d --name "$S3" -p 127.0.0.1:9092:9090 -e initialBuckets=sandbox-clustered-test adobe/s3mock:latest >/dev/null
trap 'docker rm -f "$PG" "$S3" >/dev/null 2>&1 || true' EXIT

i=0
until docker exec "$PG" pg_isready -U postgres >/dev/null 2>&1; do
  i=$((i + 1)); [ "$i" -lt 60 ] || { echo "Postgres did not start" >&2; exit 1; }
  sleep 1
done
i=0
until curl -fsS -o /dev/null http://127.0.0.1:9092/ 2>/dev/null; do
  i=$((i + 1)); [ "$i" -lt 60 ] || { echo "the S3 service did not start" >&2; exit 1; }
  sleep 1
done
# s3mock does not create buckets from its environment: create the one the test uses.
curl -fsS -X PUT http://127.0.0.1:9092/sandbox-clustered-test >/dev/null

cd "$ROOT"
SANDBOX_TEST_DATABASE_URL="postgres://postgres:$PGPASS@127.0.0.1:55432/postgres" \
SANDBOX_S3_ENDPOINT=http://127.0.0.1:9092 \
SANDBOX_S3_BUCKET=sandbox-clustered-test \
AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test AWS_REGION=us-east-1 \
  cargo test -p sandbox-server --features clustered --test clustered --test clustered_blobs --test clustered_metering --test clustered_billing -- --nocapture
