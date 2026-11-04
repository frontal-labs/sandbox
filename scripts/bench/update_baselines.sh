#!/usr/bin/env bash
# Copies benchmark results into benchmarks/baselines. It does not commit: CI no longer
# writes baselines to main, so the change goes in through a reviewed PR.
#
#   scripts/bench/update_baselines.sh <results-prefix> <baseline-prefix> <name>...
#
# Results are read from <results-prefix><name>.json and written to
# benchmarks/baselines/<baseline-prefix><name>.json (the prefix may be empty). A name without results is
# skipped. Review the diff, then commit it on a branch and open a PR.
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
else
  echo "baselines changed; review with: git diff -- benchmarks/baselines"
fi
