# Frontal Sandbox Image

_The Tier 2 guest: a headless, Ubuntu-userland Linux image built for agents, booted by Firecracker, resumed from snapshots, with optional per-thread Xfce desktops._

This document is the spec for a coding agent to build the Tier 2 image. It extends the project specification §4.9 (snapshots), §4.10 (`sandbox-vmm`, `sandbox-guest`), §4.11 (promotion) and §4.16 (CLI). Read the project specification first. Where the two disagree, the project specification's core invariants (§2) win; raise the conflict in an ADR instead of working around it.

Work milestone by milestone (§9). Do not start a milestone until the previous one's acceptance criteria pass. Statements marked **[verify]** are believed true but must be checked against the pinned Firecracker release's docs before you rely on them.

## 1. Goals and non-goals

**Goals**

- A headless guest with an Ubuntu userland and `apt`, a Firecracker-validated kernel, and `sandbox-guest` as PID 1. Bash is the interface for everything, including the desktop.
- Resuming a warm snapshot is the default path. Cold boot exists only to build snapshots.
- Package layers stored in `sandbox-cas`, keyed by their resolved package set, shared read-only across sessions, forks and VMs.
- The same filesystem layout, `PATH` and `/etc/agent/env.json` in Tier 1 and Tier 2, so promotion is invisible to the agent.
- Agent-native interfaces: capped command output, `--json` on our own commands, a machine-readable environment file, agent-callable `sandbox fork` and `sandbox undo`.
- An optional desktop layer: Xfce on X11, one VM per agent thread, forked from a warm desktop snapshot, driven by a `cua` command.
- Every resumed or forked VM is unique: entropy, IDs, clocks, credentials.

**Non-goals (v1)**

- GPU, video acceleration, WebGL-heavy workloads. Firecracker has no GPU or display device.
- Wayland. Xfce's Wayland compositor (xfwl4) had its first preview release on 2026-06-21; revisit when it reaches parity.
- `snapd` and snap packages, systemd, sshd, cron, any long-running daemon outside `sandbox-guest`'s process tree.
- A host OS, bare-metal installer, or hardware drivers. The guest only sees virtio devices.
- A shared X server across agent threads. X11 does not isolate clients on the same server.
- Our own kernel-level input driver beyond XTEST/uinput. Cua Driver integration is evaluated in I5, not built.
- arm64 images. x86_64 first; arm64 after I4.
- Persisting running processes across promotion (unchanged from the project specification).

## 2. Architecture overview

```
host
  scheduler ── places forks on the host holding the parent's memory file
  worker
    sandbox-vmm ── jailer + firecracker (pinned ≥ v1.16.1)
       kernel:   vmlinux 6.18 (built in, no modules)
       drives:   vda  base layer        (ro, from CAS)
                 vdb… package layers    (ro, from CAS)
                 vdN  session overlay   (rw, from vfs.export_ext4)
       vsock:    host dials guest port SANDBOX_GUEST_PORT
       net:      tap in a per-VM network namespace, egress via sandbox-policy proxy
guest
  sandbox-guest (PID 1)
    mounts overlayfs: lower = base + layers, upper = session overlay
    listens on vsock, executes frames, reaps children
    └─ bash (per Exec)
    └─ [desktop] Xorg :0 (dummy driver) → dbus-launch startxfce4 (user agent)
```

**Core invariants**

1. The VFS is the source of truth (project specification §2). The session overlay is derived from the VFS root; package layers are CAS objects named in the session lockfile.
2. The guest never holds long-lived credentials. Secrets are attached by the host egress proxy (project specification §4.7). Nothing secret exists in a VM when it is snapshotted.
3. `sandbox-guest` is always the vsock **listener** and the host always dials. Firecracker closes open vsock connections on snapshot; listening sockets survive.
4. No process runs in the guest outside `sandbox-guest`'s tree. No timers, no periodic jobs. An idle guest uses no CPU.
5. After every snapshot load, the host sends `Reseed` before anything else. The guest rejects every other frame until `Reseed` succeeds on that connection generation.
6. Every VM gets its own network namespace. Clones share the same guest IP and MAC by design.

## 3. Repository layout additions

```
images/
├── kernel/
│   └── config-6.18-x86_64      # derived from Firecracker resources/guest_configs
├── base/
│   └── manifest.toml           # suite, archive snapshot, package set, apt prefs
├── templates/
│   ├── python-3.12/manifest.toml
│   └── node-20/manifest.toml
└── desktop/
    ├── manifest.toml
    └── xorg-dummy.conf
crates/
├── sandbox-image/              # layer builder, lockfile resolver, layer IDs (host side)
├── sandbox-cua/                # `cua` binary: X11 control via x11rb (static musl)
└── sandbox-guest/              # extended: PID 1 duties, Reseed, EnvInfo, desktop launcher
tools/
└── image-build/                # thin wrapper: build kernel + layers in CI
docs/adr/
├── NNNN-package-layers.md      # I0
└── NNNN-guest-protocol-v1.md   # I0
```

## 4. Specifications

### 4.1 Kernel

- Linux 6.18, the newest guest kernel in Firecracker's kernel support policy (supported from v1.16.1, minimum end of support 2028-06-01). 6.1's minimum support ended 2026-09-02.
- Start from Firecracker's published guest configs. The policy says those configs target Amazon Linux `microvm-kernel-*` sources; build from those tags, or document every divergence if you build from mainline.
- Required: `VIRTIO_BLK`, `VIRTIO_NET`, `VIRTIO_VSOCKETS`, `HW_RANDOM_VIRTIO`, `VIRTIO_BALLOON`, `ACPI` + `PCI` (needed for ACPI boot on x86_64), `VMGENID`, `OVERLAY_FS`, `EROFS_FS`, `EXT4_FS`, `KVM_GUEST`, `SECCOMP`, `SECURITY_LANDLOCK`, `CGROUPS` (v2). Desktop: `INPUT_UINPUT`.
- `CONFIG_MODULES=n`. Everything built in.
- Disable legacy boot: `X86_MPPARSE=n`, `VIRTIO_MMIO_CMDLINE_DEVICES=n` (deprecated by Firecracker).
- `IO_URING`: default off. **Decide by test**: run the agent-transcript corpus with it off; enable only if a common tool fails.
- PCI transport (`--enable-pci`) is optional in Firecracker and claims better virtio throughput. **Decide by benchmark** in I2: resume latency and block throughput, MMIO vs PCI.
- Boot args: `reboot=k panic=1 init=/sbin/sandbox-guest` plus root/overlay parameters. No serial console in production builds.

### 4.2 Package layers and lockfile

This replaces "install = mount a hash" from earlier drafts. Debian packages run maintainer scripts (`postinst`, triggers, `ldconfig`, alternatives, users), so an unpacked `.deb` is not an installed package. Layers are therefore built by running `apt` and storing the result.

- **Layer:** a read-only filesystem image produced by running `apt-get install` for a resolved package set on top of a parent layer, inside a build sandbox, with maintainer scripts executed.
- **Layer ID:** `BLAKE3(parent_layer_id ‖ sorted[(name, version, arch, deb_sha256)] ‖ builder_version)`. Same inputs, same ID.
- **Format:** **decide by benchmark** in I3: EROFS vs ext4, measured on resume latency, read throughput and CAS dedupe ratio. Store in `sandbox-cas` with FastCDC chunking (project specification §4.2).
- **Archive pinning:** every manifest pins an archive snapshot timestamp so builds are reproducible. **[verify]** that `snapshot.ubuntu.com` serves the chosen suite; otherwise mirror the archive into object storage.
- **Suite:** one Ubuntu LTS suite per image generation, recorded in `images/base/manifest.toml`. Open question §12.
- **Snaps:** forbidden. Ship `/etc/apt/preferences.d/no-snap` pinning `snapd` to priority -1. Ubuntu's `firefox` and `chromium-browser` debs are snap transitional packages; get browsers from Playwright's managed Chromium or Mozilla's apt repository instead.
- **Lockfile:** `/etc/agent/lock.toml` lists requested `name = "version-constraint"` entries and the resolved layer chain. The host owns it; the guest reads it.
- **Runtime installs (v1):** `apt install`, `pip install` and `npm install` run normally inside the guest and write into the session overlay. All registry traffic goes through the host egress proxy, which caches artifacts in the CAS by hash, so repeat downloads are local. The host records the resolved set; at the next sleep or snapshot it may rebase the session onto an equivalent cached layer.
- **Runtime installs (later):** attaching a cached layer to a running VM depends on Firecracker block-device hotplug. **[verify]** what `docs/device-hotplug.md` supports in the pinned release before designing on it.
- **Templates:** `python-3.12` and `node-20` are layers plus a warm headless snapshot.

### 4.3 `sandbox-guest` (extends project specification §4.10)

**PID 1 duties:** mount `proc`, `sys`, `dev`, `devpts`, `cgroup2`, `tmpfs /run`; assemble the overlay root; reap zombies; handle `SIGTERM` and `SIGINT` for clean shutdown; never exit.

**Transport:** vsock, listening on `SANDBOX_GUEST_PORT` (constant in `sandbox-types`). Protobuf frames generated from `proto/` (the repo already uses `tonic`/`prost`). Length-prefixed.

**Frames (v1).** Existing frames from project specification: `Exec`, `Signal`, `ReadFile`, `WriteFile`, `ListDir`, `Sync`, `Ping`. New:

| Frame | Direction | Purpose |
| --- | --- | --- |
| `Hello { protocol_version, image_id }` | guest → host | First frame on every connection |
| `Reseed { entropy[64], machine_id, boot_id, hostname, realtime_ns, xauth_cookie? }` | host → guest | Required after every snapshot load. See §4.6 |
| `PrepareSnapshot {}` | host → guest | Quiesce before snapshot (§4.6). Guest replies when safe |
| `EnvInfo { json }` | host → guest | Writes `/etc/agent/env.json` |
| `ForkRequest { n, argv }` | guest → host | Backs `sandbox fork` (§4.5). Host applies policy and quota |
| `UndoRequest { steps }` | guest → host | Backs `sandbox undo` |
| `DesktopStart { width, height }` | host → guest | Launches Xorg + Xfce, replies on readiness |

**Rules**

- Until `Reseed` succeeds on a connection that follows a transport reset, every frame except `Hello` and `Ping` returns `NOT_READY`.
- Fuzz the frame decoder (`cargo-fuzz`), as project specification M4 requires.
- `sandbox-guest` is a static musl binary. No dynamic dependencies.

### 4.4 Networking

- One network namespace per VM on the host, with the tap device inside it. Every clone keeps the same in-guest IP and MAC, following Firecracker's `docs/snapshotting/network-for-clones.md`.
- Egress leaves the namespace only through the `sandbox-policy` proxy. nftables rules mirror policy (project specification §4.7), including the private-range and metadata blocks.
- After resume, the guest's open network connections are not guaranteed to survive (Firecracker docs). The agent sees ordinary connection errors; nothing tries to preserve them.

### 4.5 Agent-native interfaces

These work in **both tiers**. Most of the work belongs in `sandbox-session`, `sandbox-shell` and `sandbox-server`, not the image.

- **Output capping.** `ExecRequest` gains `output_limit` (default: first 8 KiB + last 8 KiB of each stream). Cut output gets a single marker line: `…[N lines omitted · full: /trace/exec/<seq>.out]`. The full stream is stored as a trace blob and readable through the fs API and inside the session.
- **`--json`.** Every command we ship (`sandbox`, `cua`) supports `--json`. There is no `pkg` command in v1: agents use `apt`, `pip` and `npm` directly (§4.2). Do not fork or patch GNU coreutils in Tier 2. Tier 1 builtins may add `--json` where cheap.
- **`/etc/agent/env.json`.** Generated by the host on create, promote and fork. Present in Tier 1 through the VFS. Schema (versioned):

```json
{
  "schema": 1,
  "session": "ses_…",
  "tier": "isolate | vm",
  "image": { "base": "<layer-id>", "layers": ["<layer-id>"] },
  "packages": [{ "name": "python3", "version": "3.12.3" }],
  "network": { "default": "deny", "allow": ["pypi.org", "github.com"] },
  "limits": { "memory_mib": 2048, "cpu_ms_budget": 600000, "wall_timeout_s": 3600 },
  "gui": { "available": false, "display": null, "width": null, "height": null },
  "commands": ["sandbox", "cua"]
}
```

- **`sandbox` command.**
  - `sandbox info [--json]` prints `env.json`.
  - `sandbox fork N -- <cmd>` creates N children from the current state, runs `<cmd>` in each, and returns child session IDs, exit codes and diff stats. Subject to per-key quotas.
  - `sandbox undo [N]` restores the VFS root from N checkpoints ago. In Tier 2 this restarts the VM from the base snapshot with the restored overlay; running processes are lost, and the command says so.
  - `sandbox checkpoint` takes a manual checkpoint.
- **Checkpoints.** Session config `checkpoint: per_exec | manual` (default `per_exec`). Each top-level exec ends with an incremental `vfs.commit()`; root hashes form the undo stack. Use VFS roots, not Firecracker diff snapshots, which are still in developer preview.
- **Checkpoint retention.** Keep the last `checkpoint_retain` roots per session (default 50) and unpin older ones. project specification §14 notes that checkpoint and blob GC is not implemented yet, so per-exec checkpoints grow storage until GC lands. Until then, I0 must at least unpin old roots and report per-session checkpoint bytes in `GET /v1/sessions/{id}`.
- **Exec user.** `Exec` runs as user `agent` (uid 1000) with passwordless `sudo`, in both the headless image and the desktop. Root is reachable by design (§5). Tier 1 reports the same user in `env.json` (`"user": "agent"`).
- **Approval gates.** Policy gains `require_approval: [{ host, methods, paths? }]`. The egress proxy holds a matching request, emits `ApprovalRequired { id, request_summary }` on the trace, and waits up to `approval_timeout_s` for `POST /v1/sessions/{id}/approvals/{aid}`. On timeout the request is denied and a `PolicyDenied` event is emitted. Enforcement is on the host, never in the guest.

### 4.6 Snapshot uniqueness (fork hygiene)

Firecracker's snapshot docs call resuming the same state more than once insecure unless unique state is de-duplicated. VMGenID reseeds only the kernel CSPRNG, with a race window. Everything else is our job.

**Before every snapshot (`PrepareSnapshot`):**

1. No credentials in the VM (invariant 2). Assert no environment variables listed as secrets in the session config remain.
2. No browser profile, cookie store or logged-in session exists in a template snapshot.
3. Delete any random-seed files (none expected without systemd; assert).
4. Close all outbound connections owned by `sandbox-guest`.
5. `sync`.

**After every snapshot load (`Reseed`), before any other frame:**

1. Mix host-provided entropy into the kernel pool with `RNDADDENTROPY`, then force `RNDRESEEDCRNG`, as Firecracker's `random-for-clones.md` recommends to close the VMGenID race.
2. Write a new `/etc/machine-id`.
3. Bind-mount a file with a new UUID over `/proc/sys/kernel/random/boot_id`.
4. Set the hostname.
5. Set the wall clock from `realtime_ns`.
6. Desktop: install a new X authorization cookie and restart nothing else.
7. Reply with the new values so the host can verify them.

**Rule:** no userspace process that seeded its own PRNG before the snapshot may produce security tokens after it. Template snapshots run only `sandbox-guest`, and for desktops, Xorg and Xfce.

### 4.7 Desktop layer

- **Packages:** `xserver-xorg-core`, `xserver-xorg-video-dummy`, a minimal Xfce set (`xfwm4`, `xfce4-panel`, `xfce4-session`, `xfdesktop4`, `xfce4-terminal`, `thunar`), `dbus-x11`, fonts. Chromium comes from Playwright's managed build. No snaps (§4.2).
- **Display:** Xorg with the dummy driver, not Xvfb. Default 1280×800 at 96 DPI, configurable per session. One display (`:0`) per VM.
- **Session:** `sandbox-guest` starts `Xorg :0 -config /etc/X11/xorg-dummy.conf`, then `dbus-launch startxfce4` as user `agent`. There is no systemd and no logind. Readiness means the window manager owns the screen and the panel is mapped. `DesktopStart` replies only then.
- **Template snapshot:** after readiness, the host runs `PrepareSnapshot` and takes the desktop snapshot. Threads fork from it.
- **One VM per agent thread.** Forks map the template memory file copy-on-write; that file must stay on the host for as long as any fork lives. The scheduler places forks on the host holding the file, or copies it first. Expect cross-host forks to be several times slower.
- **Idle:** pause a desktop VM after `desktop_idle_s` (default 300) with no `cua` or `exec` activity. Snapshot it to disk and resume on the next call.
- **Memory reclaim:** **decide by benchmark** in I4: Firecracker's balloon device vs. nothing, measured on resident memory per idle desktop.

### 4.8 `cua` command (`crates/sandbox-cua`)

A static binary using `x11rb` (pure-Rust X11) with the XTEST extension for input, and `GetImage` plus a PNG encoder for screenshots. Works the same whether run by the agent inside the VM or by the host through `Exec`.

| Command | Behavior |
| --- | --- |
| `cua screenshot [--region x,y,w,h] [--scale WxH] [-o file]` | PNG to stdout or file. Records the last full-frame size for scaling |
| `cua click X Y [--button left\|right\|middle] [--double] [--from WxH]` | `--from` scales coordinates from screenshot space to the live display at run time |
| `cua move X Y`, `cua drag X1 Y1 X2 Y2`, `cua scroll up\|down N` | Same scaling rules |
| `cua key <combo>` | Releases all modifiers, presses the modifiers, presses the key, releases in reverse |
| `cua type <text>` | Up to 200 chars by keystroke. Longer text goes through the clipboard, with `cua` owning the X selection in a detached child that never holds the caller's stdout |
| `cua wait-change [--timeout s]` | Returns when the screen differs from the last screenshot |
| `cua batch` | Reads JSONL actions from stdin. Prints `STEP_OK <n>` after each; on failure, names the failing step |

All commands support `--json`. Input failures exit non-zero. A key press that cannot be delivered must not report success.

**Decide by test** in I4: XTEST vs uinput for input reliability across Chromium, GTK and Xfce apps. Ship whichever passes the conformance suite (§7).

## 5. Security model

Inherits project specification §5. Additions:

- Root inside the guest is allowed. The boundary is KVM, Firecracker and the jailer.
- Firecracker trusts snapshot and memory files. Never load a snapshot file that did not come from our own CAS with a verified hash. Users cannot upload snapshots.
- No `snapd`, no sshd, no inbound network. A human desktop viewer (I5) tunnels over vsock through the host and requires an authenticated session key.
- No kernel modules. Kernel config changes require an ADR.
- Fuzz the guest frame decoder and the `cua batch` JSONL parser.
- Escape tests from project specification §5 must also pass inside Tier 2.

## 6. Performance targets

Targets from project specification are binding. Targets marked **(new)** are proposals; record the first real measurement in `docs/decisions/` and adjust by ADR.

| Metric | Target |
| --- | --- |
| Headless resume from snapshot | < 150 ms p50 (project specification) |
| Promotion, isolate → VM | < 400 ms p50 (project specification) |
| Desktop fork → first screenshot | < 500 ms p50 **(new)** |
| `cua screenshot`, warm | < 300 ms p50 **(new)** |
| Idle headless guest CPU | 0 wakeups/s from guest userspace **(new)** |
| Idle headless guest RSS | measure in I1, set target by ADR |
| 50-fork uniqueness | 100% distinct values |
| Layer build | byte-identical output for identical inputs |

Benchmark under concurrency as well as sequentially. Report p50, p95 and p99. Pin host, region and image.

## 7. Testing strategy

- **Boot and smoke:** Firecracker on KVM-enabled CI runners (project specification §7). Boot to `Ping`, run `bash -lc 'python3 -V && apt-get --version'`.
- **No-daemon check:** after boot, the guest process list contains only `sandbox-guest` and its children.
- **Uniqueness:** fork one snapshot 50 ways. Assert all distinct: `getrandom` output, `/etc/machine-id`, `boot_id`, hostname, X cookie (desktop), wall clock within 1 s of host.
- **vsock reconnect:** restore, dial, assert `NOT_READY` before `Reseed` and success after.
- **Network clones:** 10 concurrent clones each reach an allowed host and fail on a denied one, verified with an HTTP response (blocked TCP can appear to connect).
- **No-snap:** `apt-get install firefox` either installs a non-snap build from the configured repo or fails with a clear message. `snapd` is never installed.
- **Reproducibility:** build each layer twice in clean environments; IDs and bytes match.
- **CUA conformance:** scripted tasks with known outcomes: open `xfce4-terminal`, type a 500-character string, verify the result by reading a file; click a target in a test HTML page in Chromium at three resolutions with `--from` scaling; drag-and-drop between two windows.
- **Transcript corpus:** run the project specification agent-transcript corpus with tier forced to `Vm`. Track pass rate per image version.
- **Approval gate:** a gated `POST` waits, is approved, and succeeds; another times out and is denied with a `PolicyDenied` event.

## 8. Dependencies (initial pin list)

Host and build: Firecracker ≥ v1.16.1 (pinned exactly), jailer, `mmdebstrap` (unprivileged rootfs builds), `erofs-utils`, `e2fsprogs`.

Rust: `tokio-vsock`, `prost`/`tonic` (already in the workspace), `nix`/`rustix`, `x11rb`, `png`, `blake3`, `toml`, `serde_json`. `evdev` only if uinput wins the I4 test.

Rule from project specification §8 applies: justify every new dependency in the PR description.

## 9. Milestones and acceptance criteria

### I0: Interfaces and ADRs (alongside project specification M3)

- ADR: package layers and lockfile (§4.2). ADR: guest protocol v1 (§4.3).
- `env.json` schema and generation in Tier 1. Output capping in `ExecRequest`. `sandbox info`, `sandbox fork`, `sandbox undo`, `sandbox checkpoint` in Tier 1. `checkpoint: per_exec`.
- **Accept:** both ADRs merged. Every Tier 1 session has a valid `/etc/agent/env.json`. A 100k-line command returns capped output with a working full-output path. `sandbox fork 3 -- sh -c 'echo $RANDOM > /r'` yields 3 sessions with independent `/r`. `sandbox undo 1` restores the previous root hash.

### I1: Headless image v0 (start of project specification M4)

- Kernel 6.18 per §4.1. Base image as one ext4 layer. `sandbox-guest` as PID 1 with existing frames, plus `Hello`.
- **Accept:** boots under the jailer to `Ping`. Smoke and no-daemon tests pass. The image build is reproducible. Idle RSS is measured and recorded.

### I2: Snapshots, uniqueness, promotion (project specification M4)

- `PrepareSnapshot`, `Reseed`, `EnvInfo`. Per-VM network namespaces. Warm pool from snapshots. Promotion through `export_ext4`.
- MMIO vs PCI transport benchmark recorded.
- **Accept:** headless resume < 150 ms p50. Promotion < 400 ms p50. Uniqueness, vsock-reconnect and network-clone tests pass. `ffmpeg` runs via auto-promotion (project specification M4).

### Gate G1: promotion rate (before I3 and I4)

- Replay the transcript corpus in `Auto` tier. Record the share of sessions that hit `NeedsVm` and which commands caused it, in `docs/decisions/`.
- A human decides from that data whether I3 and I4 proceed now or wait. Record the decision.

### I3: Layered packages and templates (project specification M4 → M5)

- `sandbox-image` layer builder, layer IDs, lockfile. EROFS vs ext4 benchmark. Host package cache in the egress proxy. Templates `python-3.12` and `node-20`.
- **Accept:** layer IDs are reproducible. A cached `pip install pandas` downloads nothing from the internet on its second run. Promotion with layers stays < 400 ms p50; **if it does not, keep one image per template and record why.** No-snap test passes.

### I4: Desktop (project specification M5)

- Desktop layer, `DesktopStart`, desktop template snapshot, per-thread forks, `cua` with `--json` and `batch`, idle pause, XTEST vs uinput test, balloon benchmark.
- VM snapshots (project specification §4.9 `VmSnapshot`) are a prerequisite. Move them into this milestone if they aren't done.
- **Accept:** desktop fork → first screenshot < 500 ms p50. CUA conformance suite passes. Uniqueness test passes for desktop forks, including X cookies. Paused desktops resume and accept `cua` input. Chromium launches without snapd.

### I5: Human-in-the-loop (project specification M6)

- Approval gates (§4.5). Authenticated desktop viewer over vsock with a separate human cursor. Evaluation of Cua Driver inside the desktop image, recorded as an ADR.
- **Accept:** approval-gate tests pass. A human can join a desktop, act, and hand back control without breaking the agent's next `cua batch`.

## 10. Instructions for the building agent

1. Follow project specification §11 in full.
2. Firecracker behavior changes between releases. Before relying on any Firecracker feature (snapshots, vsock reset, PCI, hotplug, balloon, VMGenID), read the docs for the exact pinned version and cite them in the PR.
3. Never install `snapd` or systemd, or add a background daemon to an image. If a package pulls one in, pin it out and find another package.
4. Never put credentials into a VM or a snapshot. If a test needs a secret, inject it through the egress proxy.
5. Every "decide by benchmark" or "decide by test" item produces a file in `docs/decisions/` with method, numbers and choice before the code depending on it merges.
6. Keep the guest small. Each package added to the base layer needs a one-line reason in its manifest.

## 11. Top risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Layered packages slower than a single image | Benchmark in I3; fall back to per-template images |
| Maintainer scripts behave differently in layered builds | Build layers by running real `apt`; reproducibility test; compare against single-image installs in the transcript corpus |
| Snapshot clones leak or duplicate secrets | Invariant 2, `PrepareSnapshot`, `Reseed`, 50-fork test in CI |
| Snapd or systemd creep through dependencies | apt pinning, no-daemon test, no-snap test |
| Cross-host forks slow | Fork-locality scheduling; copy the memory file ahead of time |
| Xfce assumes systemd or logind somewhere | `dbus-launch` session; conformance suite; swap a component if it fails |
| Desktop memory per thread too high | Copy-on-write fork sharing, idle pause, balloon benchmark |
| Firecracker behavior changes on upgrade | Exact version pin; re-run I2 tests on every bump |
| Few sessions ever need Tier 2 | Gate G1 before investing in I3/I4 |

## 12. Open questions (need a human)

Each question has a default. Build with the default unless a human has overridden it in this section. Do not change a default yourself; raise it in the PR.

| # | Question | Default |
| --- | --- | --- |
| 1 | Ubuntu LTS suite for the first image generation | 24.04 (noble). Move to a newer LTS only by ADR after it passes the I1–I2 tests |
| 2 | Default desktop resolution | 1280×800 at 96 DPI |
| 3 | Human desktop viewer (I5) in v1? | No. I5 starts after I4 ships |
| 4 | arm64 timeline | After I4 |
| 5 | Agent-initiated `sandbox fork` quota per API key | 8 children per call, 32 live children per session, counted against the key's existing session limit |
| 6 | Publishing images | Built images are internal (GHCR `internal`, like the SDKs today). Before any public release, check GPL source-availability for the kernel and Canonical's trademark policy for using the "Ubuntu" name |

## 12a. Conventions

- ADRs continue the existing sequence in `docs/adr/` (project specification §14 cites 0003–0010; use the next free number).
- Tier 2 tests sit behind a Cargo feature (for example `vm-tests`) and run only on KVM-enabled runners. Confirm `/dev/kvm` exists on the runner before enabling the job.
- Image builds run in their own workflow, triggered by changes under `images/**`, `crates/sandbox-guest/**`, `crates/sandbox-cua/**` and `crates/sandbox-image/**`. Never commit built kernels, layers or snapshots; publish them to GHCR or object storage, addressed by layer ID.

## 13. Verified facts this spec relies on

Checked on 2026-10-25 against public docs. Re-verify against the pinned versions.

- Firecracker guest kernels 5.10, 6.1, 6.18; 6.18 supported from v1.16.1 until at least 2028-06-01; configs target Amazon Linux microvm kernels; optional PCI transport via `--enable-pci`; Firecracker does not support PCI devices: https://github.com/firecracker-microvm/firecracker/blob/main/docs/kernel-policy.md
- Snapshots map memory files `MAP_PRIVATE` with copy-on-write; vsock connections close on snapshot while listen sockets survive; network connections not guaranteed; diff snapshots in developer preview; resuming the same state more than once is insecure without de-duplication: https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/snapshot-support.md
- VMGenID reseed with a race window; `RNDADDENTROPY` + `RNDRESEEDCRNG`; `boot_id` bind-mount: https://github.com/firecracker-microvm/firecracker/blob/master/docs/snapshotting/random-for-clones.md
- No GPU passthrough in Firecracker: https://www.pandastack.ai/blog/microvm-gpu-passthrough-explained/
- xfwl4 first preview 2026-06-21; xfwm4 continues on X11: https://wiki.xfce.org/xfwl4_faq
- Reference desktop stack (Ubuntu, Xfce, Xvfb, xdotool, x11vnc): https://docs.e2b.dev/template/examples/desktop
- CUA practicalities (coordinate scaling, clipboard typing, Xorg dummy over Xvfb, cold-desktop timings): https://useagent.org/blog/computer-use-agent-desktop-sandbox
- Same-host vs cross-host fork latency: https://sandboxreview.com/posts/snapshot-and-fork-support-across-ai-agent-sandbox-providers
- Ubuntu ships Firefox as a snap by default: https://askubuntu.com/questions/1399383/how-to-install-firefox-as-a-traditional-deb-package-without-snap-in-ubuntu-22
