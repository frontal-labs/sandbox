#!/usr/bin/env bash
# Copies benchmark results into benchmarks/baselines and commits them to master.
#
#   scripts/bench/update_baselines.sh <results-prefix> <baseline-prefix> <name>...
#
# Results are read from <results-prefix><name>.json and written to
# benchmarks/baselines/<baseline-prefix><name>.json (the prefix may be empty). A name without results is
# skipped. The commit carries [skip ci], and a push made with GITHUB_TOKEN does
# not start other workflows. The rebase lets two benchmark jobs update master in
# turn.
set -euo pipefail

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
git -c user.name="github-actions[bot]" \
  -c user.email="41898282+github-actions[bot]@users.noreply.github.com" \
  commit -q -m "chore(bench): update benchmark baselines [skip ci]"
git pull -q --rebase origin master
git push -q origin HEAD:master
echo "pushed baseline update to master"
