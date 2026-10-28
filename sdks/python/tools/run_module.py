"""Runs a Python module as the main program: `run_module.py MODULE [ARGS...]`.

Bazel runs tools from their pip packages this way (`python -m MODULE`), because a
pip package's console script is not a file that a BUILD target can name.
"""

import runpy
import sys

if len(sys.argv) < 2:
    sys.exit("usage: run_module.py MODULE [ARGS...]")

module = sys.argv[1]
sys.argv = [module] + sys.argv[2:]
runpy.run_module(module, run_name="__main__", alter_sys=True)
