# 0026: Package layers and lockfile

## Status
Accepted

## Date
2026-10-25

## Context
plans/IMAGE.md §4.2 replaces ad-hoc installs with layered packages built by running `apt` (with maintainer scripts) on top of parent layers. Layers must be content-addressed, reproducible, and shareable across sessions/forks/VMs. The host must track the resolved package set and layer chain via a lockfile (`/etc/agent/lock.toml`) that the guest reads. Runtime installs write to the session overlay in v1 and may be rebased later.

## Decision
- **Layer:** A read-only filesystem image produced by running `apt-get install` for a resolved package set on top of a parent layer, with maintainer scripts executed in a build sandbox.
- **Layer ID:** `BLAKE3(parent_layer_id ‖ sorted[(name, version, arch, deb_sha256)] ‖ builder_version)`. Same inputs produce the same ID.
- **Format:** EROFS vs ext4 will be decided by benchmark in I3 (IMAGE.md §4.2). Store layers in `sandbox-cas` with FastCDC chunking per PLAN.md §4.2.
- **Lockfile:** `/etc/agent/lock.toml` lists requested `name = "version-constraint"` entries and the resolved layer chain. The host owns it; the guest reads it.
- **Archive pinning:** Every manifest pins an archive snapshot timestamp for reproducibility. Suites are tracked per image generation.
- **Snaps:** Forbidden. `/etc/apt/preferences.d/no-snap` pins `snapd` to priority -1. Browsers come from Playwright's managed Chromium or Mozilla's apt repo.
- **Runtime installs:** `apt install`, `pip install`, `npm install` run in the guest and write into the session overlay. The host egress proxy caches artifacts in CAS by hash; the host may rebase onto cached layers later.
- **Templates:** `python-3.12` and `node-20` are layers plus a warm headless snapshot.

## Consequences
### Positive
- Deterministic, shareable package sets with deduplication via CAS.
- Clear separation between base/layer state and per-session overlay.
- Reproducible builds via archive pinning and content-addressed IDs.

### Negative
- Layer builds are heavier than mounting prebuilt images.
- Maintainer scripts must be handled correctly in build sandboxes.
- Rebasing running VMs depends on block-device hotplug (to be verified).

### Risks
- Different behavior across archive snapshots if pinning is missed.
- Snap dependencies could slip in if pinning isn't enforced.

## Alternatives Considered
- Mounting pre-unpacked debs directly (rejected: maintainer scripts/alternatives/ldconfig not applied).
- Installing at runtime into layers without rebuilds (deferred to post-I0).
