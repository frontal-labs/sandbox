# extensions

Bzlmod module extensions (`module_extension` implementations) and repository
rules used from `MODULE.bazel`.

Nothing is here yet. The `rusty_v8_archive` repository rule currently lives in
`third_party/rusty_v8/rusty_v8.bzl`, and `MODULE.bazel` loads it from there. Move
it here only if a second module needs it, so the change is not made for its own
sake.
