#!/bin/sh
# Runs fixed git scenarios with this tool (built natively) and with host git, then
# diffs the output. Each repository must also pass `git fsck --strict` under host git.
# The commit hashes are the only lines that may differ, and they are masked, because
# they depend on the clock.
#
#   tools/wasm/git/conformance.sh          needs host git and cargo
set -eu

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cargo build -q --manifest-path "$ROOT/tools/wasm/Cargo.toml" -p git_tool
TOOL="$ROOT/tools/wasm/target/debug/git_tool"

# Runs scenario $1 with the git binary $2 in directory $3, printing its output.
scenario() {
  G="$1"; D="$2"
  rm -rf "$D" && mkdir -p "$D"
  cd "$D"
  if [ "$G" = git ]; then git init -q -b main .; else "$G" init -q .; fi
  mkdir -p a/b/c && printf '1\n' > a/b/c/deep.txt && printf '2\n' > a/top.txt && printf '3\n' > root.txt
  echo "== status fresh"; "$G" status
  "$G" add a root.txt
  "$G" -c user.name=Tester -c user.email=t@example.com commit -m first >/dev/null
  echo "== porcelain clean"; "$G" status --porcelain
  printf 'more\n' >> root.txt; printf 'n\n' > new.txt; mkdir -p extra; printf 'z\n' > extra/b.txt
  echo "== porcelain dirty"; "$G" status --porcelain
  echo "== human dirty"; "$G" status
  "$G" add root.txt new.txt
  echo "== ls-files"; "$G" ls-files --stage
  echo "== write-tree"; "$G" write-tree
  echo "== hash-object"; "$G" hash-object root.txt
  echo "== delete and stage"; rm a/top.txt; "$G" add a/top.txt; "$G" status --porcelain
  echo "== errors"; "$G" add nope.txt || echo "exit=$?"
  echo "== log subjects"; "$G" -c user.name=Tester -c user.email=t@example.com commit -m "Subject" -m "Para two" >/dev/null; "$G" log --format=%s
  echo "== log oneline"; "$G" log --oneline | sed 's/^[0-9a-f]\{7\} /<sha> /'
  echo "== log default (author/date masked)"; "$G" log | sed -e 's/^commit [0-9a-f]*/commit <sha>/' -e 's/^Date: .*/Date: <date>/'
  echo "== nothing to commit"; "$G" -c user.name=Tester -c user.email=t@example.com commit -m x || echo "exit=$?"
  echo "== unknown"; "$G" frobnicate || echo "exit=$?"
}

scenario git "$WORK/host" > "$WORK/host.out" 2>&1
scenario "$TOOL" "$WORK/ours" > "$WORK/ours.out" 2>&1
if diff "$WORK/host.out" "$WORK/ours.out"; then
  echo "conformance: the tool's output matches host git"
else
  echo "conformance: DIFFERENCES above" >&2
  exit 1
fi

(cd "$WORK/ours" && git fsck --strict >/dev/null 2>&1) && echo "fsck: host git accepts the repository"
