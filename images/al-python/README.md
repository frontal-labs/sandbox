# al-python

`frontal/sandbox/python:al-3.13.1`

Python on top of the [al-base](../al-base) image. Python 3.13.1 is compiled from source and installed under `/sandbox/runtimes/python`.

Runs as the `sandbox` user (uid 1000) with passwordless sudo.

## Packages

- Python 3.13.1
- `pip` and `venv`
- `uv`
- Git 2.49.0