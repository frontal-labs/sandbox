# 0017: Content-defined chunks in a store-wide chunk area

## Status
Accepted

## Date
2026-10-19

## Context
PLAN.md §4.2 requires content-defined chunking with FastCDC for files above 64 KiB, so that an edit stores only the bytes it changed, and zstd compression. The disk store did neither:

- It split large blobs at fixed 64 KiB offsets. An insertion near the start of a file shifted every later chunk, so the edited file stored almost all of its bytes again. The `fastcdc` crate was a declared dependency and was not used.
- It wrote each chunk under the chunk directory of the blob that named it. Two blobs that shared a chunk stored it twice, so identical content did not dedupe.
- It stored chunks raw. The compression option applied only to small blobs.
- It read chunks back without checking the result against the blob's hash, so a damaged chunk returned wrong bytes.
- It wrote files in place, so a crash could leave a truncated chunk. Chunks are shared, so a truncated chunk would corrupt every blob that reused it.

Measured on the old layout, a 17-byte insertion into a 1 MiB blob added 589,851 bytes of chunk files, and the per-blob directories held 14 chunk files for 8 distinct chunks.

## Decision
- **Chunking.** Blobs above 64 KiB are split with FastCDC (v2020) at a minimum of 16 KiB, an average of 64 KiB, and a maximum of 256 KiB. Blobs at or below 64 KiB are stored whole, as before.
- **Store-wide chunk area.** Every chunk is stored once, in `<shard>/chunks/<hash[0..2]>/<hash>`, where the chunk's own hash names the file. Identical chunks from different blobs share one file.
- **Framing.** Each chunk is framed like a whole blob: a flag byte, then the raw or zstd payload, according to `PutOptions::compress`. The hash is over the uncompressed content, as §4.2 requires.
- **Manifest version 2.** A chunked blob's manifest is `{"version": 2, "chunks": [...]}`. A version 1 manifest is a bare array of hashes whose chunks are raw and live under the blob's own chunk directory. Version 1 manifests are still read. They are not rewritten, because a read must not write.
- **Verified reads.** A blob, whole or chunked, is returned only if its bytes hash to its name. Otherwise the read is an error: `blob ... does not match its hash; the store is corrupt`.
- **Atomic writes.** Files are written to `<name>.tmp-<pid>-<seq>` and renamed into place. `list_parts` ignores names that are not hashes, so a leftover temporary file is never a sweep candidate and cannot stop a sweep.
- **Ordering with the collector.** Writes happen under a session gate, and the collector ([ADR 0013](0013-blob-store-garbage-collection.md)) holds every gate for the whole sweep. A write therefore finishes before the collector can run, and what it wrote is either reachable from a root when the roots are read or is garbage. A chunk that a write reuses is never swept while that write is in progress.

## Consequences
### Positive
- An edit stores the chunks that contain it. The new bytes on disk are a small part of the blob.
- Identical chunks across blobs are stored once, so the store's size follows distinct content.
- Compressible chunks are smaller on disk.
- Corruption is an error on read, not wrong data.

### Negative
- Every read hashes the blob (BLAKE3 over the bytes). This is cheap compared with decompression, but it is new work on the read path.
- Version 1 blobs keep the per-blob layout until they are removed. They do not dedupe with new blobs.
- A crash can leave a temporary file. The collector does not remove it yet, so it takes disk space until it is removed by hand.

### Risks
- Chunk boundaries now depend on content. A change to `CDC_MIN`, `CDC_AVG`, or `CDC_MAX` changes the chunks of new blobs, so the sizes are part of the on-disk format and must not change without a new manifest version.

## Alternatives Considered
- **Rewrite version 1 manifests on read.** Rejected. It turns reads into writes, and a read-only store would fail.
- **Fixed-size chunks with compression.** Rejected. Compression alone does not stop an insertion from re-storing every later chunk.
- **Keep chunks per blob, and dedupe by checking other blobs.** Rejected. Finding a shared chunk among blobs needs an index that the store does not have, while a content-named file is its own index.

## References
- [PLAN.md §4.2](../../PLAN.md): chunking, compression, and refcount/GC.
- [ADR 0013](0013-blob-store-garbage-collection.md): mark and sweep, and the locking that orders chunk writes with sweeps.
- Tests in `crates/sandbox-cas/src/disk_store.rs`: `identical_chunks_from_different_blobs_are_stored_once`, `an_edit_stores_only_the_chunks_around_it`, `an_insertion_stores_a_small_part_of_the_blob` (property), `a_corrupt_chunk_is_an_error_not_wrong_bytes`, `a_version_one_manifest_is_still_read`, `a_temporary_file_is_not_a_sweep_candidate`.
