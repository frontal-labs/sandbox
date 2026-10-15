#!/usr/bin/env python3
"""Converts criterion output into the JSON list the benchmark regression gate reads.

    criterion_to_json.py <criterion-dir> <output.json>

Each benchmark directory under <criterion-dir> has a `new/sample.json` with
per-sample totals (`times`, in ns) and iteration counts (`iters`). The per-iteration
time of a sample is times[i] / iters[i]. The output is a list of
{"name", "mean", "p99"} objects, with times in ns per iteration. Run the benchmark
suite with the criterion directory cleared first, so each output covers one suite.
"""

import json
import math
import pathlib
import sys


def percentile(sorted_values, pct):
    # Nearest-rank percentile on an ascending list.
    rank = math.ceil(pct / 100 * len(sorted_values))
    return sorted_values[max(rank - 1, 0)]


def benchmark_name(new_dir):
    meta = new_dir.parent / "benchmark.json"
    if meta.exists():
        return json.loads(meta.read_text())["full_id"]
    return str(new_dir.parent.relative_to(new_dir.parents[2]))


def convert(criterion_dir):
    results = []
    for sample in sorted(criterion_dir.glob("**/new/sample.json")):
        data = json.loads(sample.read_text())
        per_iter = sorted(t / n for t, n in zip(data["times"], data["iters"]) if n > 0)
        if not per_iter:
            continue
        results.append(
            {
                "name": benchmark_name(sample.parent),
                "mean": sum(per_iter) / len(per_iter),
                "p99": percentile(per_iter, 99),
            }
        )
    return sorted(results, key=lambda r: r["name"])


def main():
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    criterion_dir = pathlib.Path(sys.argv[1])
    if not criterion_dir.is_dir():
        print(f"no criterion output at {criterion_dir}", file=sys.stderr)
        return 1
    results = convert(criterion_dir)
    if not results:
        print(f"no benchmark samples under {criterion_dir}", file=sys.stderr)
        return 1
    pathlib.Path(sys.argv[2]).write_text(json.dumps(results, indent=2) + "\n")
    print(f"wrote {len(results)} benchmarks to {sys.argv[2]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
