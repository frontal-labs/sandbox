# 0022: Worker isolation on Linux

## Status
Accepted. It implements PLAN §4.13 for Linux. The Tier 2 microVM (M4) is separate, and it needs KVM.

## Date
2026-10-20

## Context
A worker runs the isolate backend, which executes untrusted scripts. Until now the worker was a child process with nothing around it: no privilege lock, no seccomp filter, no namespaces, and no resource limits. PLAN §4.13 asks for `PR_SET_NO_NEW_PRIVS`, seccomp-bpf, cgroup v2 limits, and namespaces, and §5 says an isolate in the server process is not a security boundary until these exist.

## Decision
Three layers, all Linux-only, applied so that each one fails closed:

1. **Namespaces, in the child.** Before its runtime starts, the child unshares its network, IPC, UTS, and mount namespaces, and makes its mounts private so nothing propagates to the host. The child has no network interfaces.
2. **Seccomp, in the child.** A BPF denylist is installed with `seccompiler`. Denied syscalls fail with `EPERM`. The list covers process inspection (`ptrace`, `process_vm_*`), mount and namespace manipulation, BPF and perf, kernel module and boot control, keyrings, and quota and accounting calls, plus the x86 port I/O calls. `seccompiler::apply_filter` sets `NO_NEW_PRIVS` before it installs the filter. The child installs the filter before it starts any thread, because a filter applies to the calling thread and to threads created after it.
3. **cgroup v2, from the host.** The host creates a child cgroup under a delegated cgroup, sets `memory.max`, `pids.max`, and `cpu.max`, and moves the worker into it right after it starts. Removing the cgroup is retried on a helper thread, since the kernel refuses to remove a cgroup that still holds processes.

**Delegation is the operator's job.** cgroup v2 lets a cgroup's children use a controller only when the controller is enabled in the parent's `cgroup.subtree_control`, and only while the parent holds no processes directly. A host therefore needs a cgroup it owns, which is what a systemd unit with `Delegate=yes` provides. The host reads that cgroup from `SANDBOX_CGROUP_ROOT`, checks that the controllers are enabled there, and fails if they are not. It does not move processes out of the way to make one: that would change the processes of the machine it runs on.

**Fail closed.** If any layer cannot be applied, the worker does not start. `Worker::spawn` uses strict isolation on Linux, and `Isolation::none()` is explicit. Off Linux, only the no-isolation worker can start.

## Consequences
### Positive
- Each layer is checked from outside the worker by a test on a real Linux kernel: `NoNewPrivs`, the seccomp mode, distinct namespaces, the cgroup limit files, and cleanup on drop. A fork-based test checks that denied syscalls return `EPERM` and that ordinary calls still work.
- The cost of the layers on a running command has not been measured. It is open, and it should be measured before the allowlist is chosen.

### Negative
- The denylist allows any syscall it does not name. An allowlist is stronger, and it needs the set of syscalls the isolate uses, measured under load. That is the next step.
- A worker still shares the host's kernel. Namespaces and seccomp reduce what a compromised script can reach, but they are not a substitute for a VM. That is Tier 2.
- Delegation has to be set up by the operator, so a host that is not delegated cannot start a worker. This is deliberate.

## Alternatives Considered
- **Set up cgroups by moving every process out of the host's cgroup.** Rejected. It changes the processes of the machine the server runs on, and the server cannot know what else lives in that cgroup.
- **Allowlist seccomp now.** Rejected for this change. It needs measurement first, and a wrong allowlist breaks scripts without a clear error.
- **Set the limits after the child has started.** Considered. The window is the child's start-up, before any script can run, so the child is limited before it serves a request. The cgroup is created before the child starts.

## Verification
- `cargo test -p sandbox-worker` in the Linux container: the seccomp fork tests pass.
- `cargo test -p cli --test worker_isolation`, in the Linux container with a delegated cgroup: six tests pass (filter, namespaces, cgroup limits, commands run, cleanup on drop, and an unisolated worker reports no isolation).
- The macOS build compiles, and the non-Linux branches return `Unsupported` for any layer.

## References
- [PLAN.md §4.13, §5](../../PLAN.md).
- `crates/sandbox-worker/src/isolation.rs`, `crates/sandbox-worker/src/worker.rs`, `cli/tests/worker_isolation.rs`.
- `docker/run-linux.sh`, `docker/linux-dev.Dockerfile`.
