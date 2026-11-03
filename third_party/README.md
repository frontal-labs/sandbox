# Third-Party Dependencies

This directory contains vendored third-party dependencies that are not available via standard package managers or require specific build configurations.

## rusty_v8

**Path**: `third_party/rusty_v8/`

High-quality Rust bindings for Google's V8 JavaScript engine, maintained by the [denoland](https://github.com/denoland/rusty_v8) organization.

### Contents

- `BUILD.bazel` - Bazel build rules for compiling V8 and the Rust bindings
- `rusty_v8.bzl` - Bazel module definition and repository rules

### Usage

This dependency is consumed via Bazel. The `rusty_v8` crate provides safe Rust APIs for:

- Creating V8 isolates and contexts
- Compiling and executing JavaScript code
- Interoperating between Rust and JavaScript values
- Working with V8's garbage collector (HandleScopes, Persistent handles)

### Versioning

The version is pinned in the Bazel module definition (`rusty_v8.bzl`). Update by modifying the commit hash or tag reference in that file.

### Building

Requires Bazel 6+. The V8 engine is built from source as part of the rusty_v8 build process.

```bash
bazel build //third_party/rusty_v8:rusty_v8
```

### License

V8: BSD-3-Clause
rusty_v8 bindings: MIT