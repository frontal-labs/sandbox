# 0026: Package layers and lockfile

## Status
Proposed

## Date
2026-10-25

## Context
plans/IMAGE.md §4.2 replaces "install means mount a hash" with layers built by running `apt`. Debian packages run maintainer scripts (`postinst`, triggers, `ldconfig`, alternatives, users), so an unpacked `.deb` is not an installed package. A layer is therefore the result of running `apt-get install` for a resolved package set on top of a parent layer, inside a build sandbox.

Layers must be content addressed, shared read-only across sessions, forks and VMs, and reproducible: the same inputs give the same ID and, for the same storage format, the same bytes. The host must record which packages a VM was built with, in a file the guest can read.

Nothing in the repository implements layers yet. IMAGE §3 names `sandbox-image` as the owner, and it does not exist. This ADR fixes the identity and the lockfile shape so that I1 to I3 build against them. It does not choose the storage format. IMAGE §4.2 assigns that to a benchmark in I3.

## Decision
- **Layer.** A read-only filesystem image produced by running `apt-get install` for a resolved package set on top of a parent layer. The base layer has no parent.
- **Layer ID.** `BLAKE3(parent ‖ packages ‖ builder)`, with a fixed encoding so that two implementations agree on the bytes:
  - `parent` is the 32-byte ID of the parent layer. The base layer uses 32 zero bytes.
  - `packages` is the list of `(name, version, arch, deb_sha256)` tuples. Each tuple is encoded as its four fields, each a `u32` big-endian length followed by the UTF-8 bytes (the digest as 32 raw bytes). The encoded tuples are sorted bytewise and concatenated.
  - `builder` is `builder_version`, a length-prefixed UTF-8 string. It names the build tooling, its version, and the environment the build pins (locale, timezone, `umask`, and the apt and mmdebstrap versions). A change to any of them makes a new layer ID.
- **Storage format is not part of the ID.** A layer ID names the installed package set. The stored artifact is keyed by `(layer ID, format)`. Choosing EROFS or ext4 in I3 therefore does not change any ID. Reproducibility tests compare artifacts of the same format.
- **Storage.** Layer artifacts live in `sandbox-cas`, chunked with FastCDC above 64 KiB and compressed with zstd (PLAN §4.2). Layer artifacts are never committed to git.
- **Archive pinning.** `images/base/manifest.toml` names the Ubuntu suite and an archive snapshot timestamp. Both are required, and a build without them fails. The suite default is `noble` (24.04), per IMAGE §12 question 1. The snapshot host must serve that suite: **not verified here** (IMAGE §13 marks it [verify]). If it does not, the archive is mirrored into object storage and the mirror becomes the source.
- **No snaps.** The base layer ships `/etc/apt/preferences.d/no-snap`, which sets `Pin-Priority: -1` for `snapd`. The builder also rejects any resolved package set that contains `snapd`, so a dependency cannot bring it in.
- **Lockfile.** `/etc/agent/lock.toml` is written by the host, is read-only in the guest, and records the request and the resolved chain. Schema v1:

  ```toml
  schema = 1
  suite = "noble"
  archive_snapshot = "20261001T000000Z"
  base = "<layer-id>"

  [requested]            # name = "version-constraint", as the caller asked
  python3 = ">=3.12"

  [[layers]]             # resolved, in build order
  id = "<layer-id>"
  parent = "<layer-id>"
  packages = [{ name = "python3", version = "3.12.3-0ubuntu2", arch = "amd64", deb_sha256 = "<hex>" }]
  ```

  The file holds no timestamps, so identical inputs give identical bytes. A Tier 1 session has no layers in I0, so it gets no lockfile yet. The lockfile is first written in I3.
- **Runtime installs (v1).** `apt`, `pip` and `npm` run in the guest and write into the session overlay. They do not change the lockfile, which describes the layers the VM started from. The egress proxy caches registry artifacts by hash. Rebasing a running VM onto a cached layer depends on block-device hotplug. That is [verify] in the pinned Firecracker release and is out of scope here.
- **Templates.** `python-3.12` and `node-20` are a layer chain plus a warm headless snapshot (IMAGE §4.2).

## Consequences
### Positive
- Identical inputs give identical IDs, so the same package set is built once and shared by every session, fork and VM that needs it.
- The encoding is fixed in the ADR. Two builders cannot disagree about an ID.
- Keeping the format out of the ID lets the I3 benchmark change the format without renaming any layer.

### Negative
- A layer costs a full `apt-get install` run. Builds are slower than mounting a prebuilt image.
- Maintainer scripts must run inside the build sandbox, so the sandbox must be able to run them. That is the reason the builder needs its own pinned environment.
- Runtime installs are not recorded in the lockfile, so the lockfile can differ from what a VM actually has. The trace is the place to record them, and that record is not built yet.

### Risks
- A missed archive pin makes two builds of the same request differ. The manifest rule and the reproducibility test guard against it.
- The no-snap rule holds only if every dependency is checked. The builder check is the enforcement point. The apt pin is a second, weaker guard.

## Alternatives Considered
- **Mount pre-unpacked `.deb` files.** Rejected. Maintainer scripts, alternatives and `ldconfig` would not run.
- **Put the storage format in the layer ID.** Rejected. It would rename every layer when the I3 benchmark picks a format, and the ID would then depend on a storage choice rather than on the installed content.
- **One image per template, no layers.** Kept as the fallback. IMAGE §9 I3 says to use it if layered promotion misses its 400 ms target.
- **Install at runtime into the session overlay only.** Kept for v1 runtime installs. Layers are for packages a template needs before the VM starts.

## References
- plans/IMAGE.md §4.2, §9 (I3), §12 (questions 1 and 6), §13
- plans/PLAN.md §4.2 (CAS chunking and compression)
- [ADR 0017](0017-content-defined-chunks.md): content-defined chunks in the CAS
- [ADR 0027](0027-guest-protocol-v1.md): the guest reads the lockfile through the protocol
