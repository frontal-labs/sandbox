"""Records GNU coreutils output for `coreutils_cases.py` into `coreutils.json`.

Run from this directory: `python3 record_coreutils.py`. The GNU tools come from
Homebrew (`brew install coreutils gnu-sed gawk grep findutils diffutils gnu-tar gzip`).
Their `gnubin` directories are put first on PATH, so `sort` and `sed` mean the GNU
versions. Each case runs in a fresh empty directory, so relative paths behave the same
as they do in the sandbox, which starts at `/`. The Rust test replays the result.
"""

import json
import os
import subprocess
import tempfile

from coreutils_cases import CASES

BASH = "bash"
BREW = "/opt/homebrew/opt"
GNU_DIRS = [
    f"{BREW}/coreutils/libexec/gnubin",
    f"{BREW}/gnu-sed/libexec/gnubin",
    f"{BREW}/gawk/libexec/gnubin",
    f"{BREW}/grep/libexec/gnubin",
    f"{BREW}/findutils/libexec/gnubin",
    f"{BREW}/diffutils/bin",
    f"{BREW}/gnu-tar/libexec/gnubin",
    f"{BREW}/gzip/bin",
]


def gnu_path() -> str:
    return ":".join(GNU_DIRS + [os.environ["PATH"]])


def gnu_version() -> str:
    out = subprocess.run(["sort", "--version"], capture_output=True, text=True,
                         env={"PATH": gnu_path()}, check=True)
    return out.stdout.splitlines()[0]


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
    for name, script in CASES:
        records.append({"name": name, "script": script, **run(script)})
    out = {"recorded_with": gnu_version(), "cases": records}
    with open(os.path.join(os.path.dirname(__file__), "coreutils.json"), "w", encoding="utf-8") as f:
        json.dump(out, f, indent=1, ensure_ascii=False)
        f.write("\n")
    print(f"recorded {len(records)} cases with {out['recorded_with']}")


if __name__ == "__main__":
    main()
