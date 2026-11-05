# 0035: Blobs in an object store

## Status
Accepted

## Date
2026-11-05

## Context
A durable server keeps its blobs in a directory on its own disk (ADR 0008). A node that does not
have a blob cannot read it, so a second node cannot serve the sessions of the first, and the blobs
are lost when the disk is. The plan calls for an S3-compatible object store behind the content
store.

## Decision
A durable server can keep every blob in an S3 bucket too, with `serve --data-dir <dir>
--object-store-bucket <name>`, and `--object-store-endpoint <url>` for an S3-compatible service.
Credentials come from the `AWS_*` variables.

- The blob store is the disk store wrapped in `TieredStore` with the write-through policy. A write
  goes to the disk and to the bucket. A read tries the disk, then the bucket, and keeps what it
  reads on the disk. A delete removes the blob from both.
- A put checks the bucket on every call, not only when the blob is new to the disk. A blob written
  before the bucket was configured, or whose bucket write failed, therefore reaches the bucket on
  its next put. Without this a blob could stay local-only.
- The choice is fixed for the process, as the directory is (ADR 0008). Reopening with a different
  directory, or with or without a bucket, is refused.
- A clustered server takes the same option. Every node writes its blobs to the bucket, and a node
  without a blob reads it from there, so the blob directory no longer has to be shared.

## Consequences
- A blob written on one node reads on another that has none of it locally, and a delete reaches
  both. Both are tested against an S3-compatible service (`docker/object-store-test.sh`, which runs
  `adobe/s3mock`). A smoke run also confirmed that a file read after its local blobs were deleted
  and the server restarted came back from the bucket.
- The write path makes one bucket request per put to check for the blob, and a write when it is
  missing. Writes are therefore slower than local-only ones. A batch or cache for those checks is a
  possible later change.
- The clustered round trip is tested with Postgres and adobe/s3mock (`docker/clustered-test.sh`).
- The tests ran against adobe/s3mock, not MinIO. MinIO's images could not be pulled from Docker Hub
  or Quay here. The tests use only the S3 API, so MinIO should behave the same, but it has not been
  run.
- The chunked-blob index stays local. A node that loses its disk reads whole blobs from the bucket,
  but the chunk files are not in the bucket and are rebuilt only as blobs are read again.
