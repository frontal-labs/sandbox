# git_tool

A git subset for the sandbox's WASI tool registry (PLAN.md section 4.6). It writes real
repositories: loose objects, the v2 index, and refs under `.git`. Host git reads them, and
`git fsck --strict` accepts them.

## Commands

| Command | Notes |
| --- | --- |
| `init [-q] [dir]` | Creates `.git`. The branch is always `main`. |
| `add <path>...` | Stages files and directories. A tracked file that is gone is staged as deleted. `add .` stages everything. |
| `commit [-q] -m <msg> [-m <para>]... [--author="Name <email>"]` | Needs `-c user.name=` and `-c user.email=`, or uses the default identity below. |
| `status [--porcelain] [-b]` | Porcelain v1 matches host git. The long form matches for the cases in the conformance script. |
| `log [--oneline] [--format=%H\|%h\|%s] [-n N]` | Walks the first parent. |
| `write-tree`, `ls-files --stage`, `hash-object [-w] <file>` | Plumbing, used for conformance. |

`clone`, `fetch`, `pull`, `push`, and `remote` refuse with a message, because the sandbox
has no network access for git.

## Differences from host git

- **No environment.** The WASI host passes argv only. Identity therefore comes from
  `-c user.name=` and `-c user.email=`, and falls back to `Sandbox <sandbox@frontal.local>`.
  Host git would refuse to commit without an identity.
- **`init` prints `.git/`** and not the absolute path, because the guest does not know its
  host path.
- **Stat data is not stored.** The index's times, device, inode, uid, and gid are zero.
  Status compares contents, so the output is the same. Host git rechecks the contents too.
- **No pack files, no `.gitignore`, no symlinks, no submodules, no detached HEAD.**
  Repositories that host git packed are refused with an error.
- **Commit messages** are not stripped of comments. Trailing whitespace is trimmed, and a
  newline is added, as git does.
- **Commit output** prints the `[main ...] subject` line but not the diffstat.
- **Paths** must be valid UTF-8, and porcelain output does not quote unusual characters.

## Verification

- `tools/wasm/git/conformance.sh` runs fixed scenarios against host git 2.52 and diffs the
  output. Commit hashes are masked, because they depend on the clock. It also runs
  `git fsck --strict` on the repository the tool wrote.
- `crates/sandbox-wasm/tests/git_conformance.rs` runs the built module inside the sandbox and
  compares the object hashes, index, tree, and porcelain status with host git's values.
- `crates/sandbox-wasm/tests/wasi_fs.rs` covers the command contract.

## Building

`tools/wasm/build.sh` builds the module, prints its BLAKE3 hash, and installs it as
`crates/sandbox-wasm/fixtures/git_tool.wasm`. The hash is pinned in
`crates/sandbox-wasm/fixtures/tools/tools.toml`.
