#!/usr/bin/env bash
# Copies benchmark results into benchmarks/baselines and commits them to main.
#
#   scripts/bench/update_baselines.sh <results-prefix> <baseline-prefix> <name>...
#
# Results are read from <results-prefix><name>.json and written to
# benchmarks/baselines/<baseline-prefix><name>.json (the prefix may be empty). A name without results is
# skipped. The commit carries [skip ci], and a push made with GITHUB_TOKEN does
# not start other workflows. The rebase lets two benchmark jobs update main in
# turn.
set -euo pipefail

# Every git command here needs an identity, including the rebase below.
export GIT_AUTHOR_NAME="github-actions[bot]"
export GIT_AUTHOR_EMAIL="41898282+github-actions[bot]@users.noreply.github.com"
export GIT_COMMITTER_NAME="$GIT_AUTHOR_NAME"
export GIT_COMMITTER_EMAIL="$GIT_AUTHOR_EMAIL"

results_prefix="${1:?results prefix}"
baseline_prefix="${2-}"
shift 2

mkdir -p benchmarks/baselines
for name in "$@"; do
  src="${results_prefix}${name}.json"
  if [[ ! -f "$src" ]]; then
    echo "no results for ${name}; skipping"
    continue
  fi
  cp "$src" "benchmarks/baselines/${baseline_prefix}${name}.json"
  echo "updated benchmarks/baselines/${baseline_prefix}${name}.json"
done

if [[ -z "$(git status --porcelain -- benchmarks/baselines)" ]]; then
  echo "baselines unchanged"
  exit 0
fi

git add benchmarks/baselines
git commit -q -m "chore(bench): update benchmark baselines [skip ci]"
git pull -q --rebase origin main
git push -q origin HEAD:main
echo "pushed baseline update to main"
