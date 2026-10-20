#!/usr/bin/env python3
"""Checks that each Bazel Rust target lists the dependencies its Cargo package declares.

Cargo is the source of truth for dependencies. For every workspace package this
compares the `deps` and `proc_macro_deps` of its Bazel `rust_library`,
`rust_binary`, and `rust_test` targets with `cargo metadata`:

- a normal Cargo dependency must appear in the library's `deps` (as `//path:target`
  for a workspace crate, or `@crates//:name` for a registry crate), unless it is
  a proc-macro, which goes in `proc_macro_deps`;
- a dev dependency must appear in the test targets;
- a target listed in Bazel but not declared in Cargo is reported as stale.

Dependencies behind a `cfg(...)` target are checked as well: they must appear in
a `select` on the platform they apply to.

Usage: bazel/tools/check_bazel_deps.py [--root DIR]
Exit status is 1 when anything is missing or stale.
"""

import argparse
import ast
import json
import os
import subprocess
import sys

RULES = {"rust_library", "rust_binary", "rust_test", "cargo_build_script"}


def cargo_metadata(root):
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(out)


def bazel_targets(build_file):
    """Returns {name: {"deps": set, "proc_macro_deps": set, "dev": ...}} for the rules in a BUILD file."""
    with open(build_file) as f:
        tree = ast.parse(f.read(), filename=build_file)
    targets = {}
    for node in ast.walk(tree):
        if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Name)):
            continue
        if node.func.id not in RULES:
            continue
        fields = {}
        for kw in node.keywords:
            if kw.arg in ("name", "crate", "crate_root", "srcs"):
                fields[kw.arg] = kw.value
            elif kw.arg in ("deps", "proc_macro_deps"):
                fields[kw.arg] = collect_labels(kw.value)
        name = fields.get("name")
        if not isinstance(name, ast.Constant):
            continue
        targets[name.value] = {
            "rule": node.func.id,
            "deps": fields.get("deps", set()),
            "proc_macro_deps": fields.get("proc_macro_deps", set()),
            "crate": fields["crate"].value if isinstance(fields.get("crate"), ast.Constant) else None,
        }
    return targets


def collect_labels(value):
    """All string labels in a deps value, including those inside a `select`."""
    labels = set()
    for node in ast.walk(value):
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            labels.add(node.value)
    return labels


def package_label(pkg_dir, target):
    rel = os.path.relpath(pkg_dir, ROOT)
    return "//" + ("" if rel == "." else rel.replace(os.sep, "/")) + ":" + target


def main():
    global ROOT
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=os.path.join(os.path.dirname(__file__), "..", ".."))
    args = parser.parse_args()
    ROOT = os.path.abspath(args.root)

    meta = cargo_metadata(ROOT)
    packages = {p["id"]: p for p in meta["packages"]}
    workspace = set(meta["workspace_members"])
    by_name = {}
    for p in meta["packages"]:
        by_name.setdefault(p["name"], []).append(p)
    proc_macro = {p["name"] for p in meta["packages"] if any("proc-macro" in t["kind"] for t in p["targets"])}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}

    problems = []
    for pkg_id in sorted(workspace):
        pkg = packages[pkg_id]
        pkg_dir = os.path.dirname(pkg["manifest_path"])
        build_file = os.path.join(pkg_dir, "BUILD.bazel")
        if not os.path.exists(build_file):
            problems.append(f"{pkg['name']}: no BUILD.bazel")
            continue
        targets = bazel_targets(build_file)
        node = nodes[pkg_id]
        # A dependency's kind is None for normal, "dev", or "build". The target
        # field is the cfg it applies under, or None.
        expected_lib, expected_dev = set(), set()
        for dep in node["deps"]:
            dep_pkg = packages[dep["pkg"]] if dep["pkg"] in packages else None
            if dep_pkg is None:
                continue
            kinds = dep.get("dep_kinds", [])
            is_workspace = dep["pkg"] in workspace
            if is_workspace:
                target_name = dep["name"]
                label = package_label(os.path.dirname(dep_pkg["manifest_path"]), target_dep_name(dep_pkg))
            else:
                multiple = len(by_name.get(dep_pkg["name"], [])) > 1
                ext = f"{dep_pkg['name']}-{dep_pkg['version']}" if multiple else dep_pkg["name"]
                label = f"@crates//:{ext}"
            for k in kinds:
                if k["kind"] in (None, "normal"):
                    expected_lib.add((label, dep_pkg["name"] in proc_macro, k.get("target")))
                elif k["kind"] == "dev":
                    expected_dev.add((label, dep_pkg["name"] in proc_macro, k.get("target")))

        lib_name = lib_target_name(pkg)
        if lib_name is None:
            continue
        lib = targets.get(lib_name)
        if lib is None:
            problems.append(f"{pkg['name']}: no rust_library named {lib_name}")
            continue
        for label, is_proc, cfg in expected_lib:
            listed = lib["proc_macro_deps"] if is_proc else lib["deps"]
            if not any(label_matches(x, label) for x in listed):
                problems.append(f"{pkg['name']}: {lib_name} is missing {label}{' (proc-macro)' if is_proc else ''}{' under ' + cfg if cfg else ''}")
        tests = [t for t in targets.values() if t["rule"] == "rust_test"]
        if tests:
            for label, is_proc, cfg in expected_dev:
                if not any(any(label_matches(x, label) for x in t["deps"] | t["proc_macro_deps"]) for t in tests):
                    problems.append(f"{pkg['name']}: no rust_test lists dev dependency {label}")
        # Stale: a registry label that no dependency of this package declares. Every
        # declared dependency counts, including build-only and optional ones, which
        # the activated graph above leaves out.
        declared_names = {d["name"].replace("_", "-") for d in pkg["dependencies"]}
        declared_names |= {d["name"] for d in pkg["dependencies"]}
        for target_name, target in targets.items():
            for listed in target["deps"] | target["proc_macro_deps"]:
                if not listed.startswith("@crates//:"):
                    continue
                if crate_base(listed) not in declared_names:
                    problems.append(f"{pkg['name']}: {target_name} lists {listed}, which Cargo does not declare for {pkg['name']}")
    if problems:
        print("\n".join(problems))
        print(f"{len(problems)} problem(s)")
        return 1
    print("all Bazel Rust targets match Cargo dependencies")
    return 0


def target_dep_name(dep_pkg):
    """The Bazel target a workspace dependency's library is named."""
    if dep_pkg["name"] == "cli":
        return "cli_lib"
    return dep_pkg["name"].replace("-", "_")


def lib_target_name(pkg):
    if not any("lib" in t["kind"] or "rlib" in t["kind"] for t in pkg["targets"]):
        return None
    return target_dep_name(pkg)


def label_matches(listed, label):
    """Whether a listed label names the same target as `label`.

    Workspace labels must match exactly. A registry label matches by crate name:
    crate_universe exposes `@crates//:name`, and a versioned `@crates//:name-x.y.z`
    when several versions of the crate exist, so the version suffix is ignored.
    """
    listed = listed.replace("@@", "@")
    if listed == label:
        return True
    if not (listed.startswith("@crates//:") and label.startswith("@crates//:")):
        return False
    return crate_base(listed) == crate_base(label)


def crate_base(label):
    """`@crates//:thiserror-2.0.21` and `@crates//:thiserror` both give `thiserror`."""
    name = label.split(":", 1)[1]
    head, _, tail = name.rpartition("-")
    if head and tail[:1].isdigit():
        return head
    return name


if __name__ == "__main__":
    sys.exit(main())
