"""Records the expected output of each conformance case under GNU bash.

Run from this directory: `python3 record.py`. The GNU tools from Homebrew come first on
PATH (see `record_coreutils.py`), so every command means its GNU version. Each case runs in a fresh empty
directory, so relative paths behave the same as they do in the sandbox, which
starts at `/`. The output is written to `cases.json`, which the Rust test
replays. Re-run this script whenever a case changes, and commit the result.
"""

import json
import os
import subprocess
import tempfile

from cases import cases
from record_coreutils import gnu_path, gnu_version

BASH = "bash"


def bash_version() -> str:
    out = subprocess.run([BASH, "--version"], capture_output=True, text=True, check=True)
    return out.stdout.splitlines()[0] + "; " + gnu_version()


def run(script: str) -> dict:
    with tempfile.TemporaryDirectory() as work:
        proc = subprocess.run(
            [BASH, "-c", script],
            cwd=work,
            capture_output=True,
            timeout=10,
            env={"PATH": gnu_path(), "LC_ALL": "C"},
        )
    return {
        "stdout": proc.stdout.decode("utf-8", errors="replace"),
        "status": proc.returncode,
    }


def main() -> None:
    records = []
    for name, script in cases():
        expected = run(script)
        records.append({"name": name, "script": script, **expected})
    out = {"recorded_with": bash_version(), "cases": records}
    with open(os.path.join(os.path.dirname(__file__), "cases.json"), "w", encoding="utf-8") as f:
        json.dump(out, f, indent=1, ensure_ascii=False)
        f.write("\n")
    print(f"recorded {len(records)} cases with {out['recorded_with']}")


if __name__ == "__main__":
    main()
