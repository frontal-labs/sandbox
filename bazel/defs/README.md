# defs

Starlark definitions that the workspace's BUILD files load: macros and rule
wrappers that several packages share.

Nothing is here yet. Add a `.bzl` file when two or more BUILD files repeat the
same rule setup. Load it with `load("//bazel/defs:name.bzl", ...)`.
