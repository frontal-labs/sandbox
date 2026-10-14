"""Fetches the prebuilt static V8 library that the `v8` crate links against.

The `v8` build script normally downloads this archive itself. Under Bazel it
writes the file outside the declared build outputs, so the link step cannot see
it. This rule fetches the same release asset into a stable external repo, and
the `v8` crate annotation in MODULE.bazel adds it to the rlib's inputs and link
search path.
"""

def _rusty_v8_archive_impl(rctx):
    rctx.download(
        url = rctx.attr.url,
        output = "librusty_v8.a.gz",
        sha256 = rctx.attr.sha256,
    )
    result = rctx.execute(["gzip", "-d", "librusty_v8.a.gz"])
    if result.return_code != 0:
        fail("failed to decompress librusty_v8.a.gz: " + result.stderr)
    rctx.file(
        "BUILD.bazel",
        'package(default_visibility = ["//visibility:public"])\n' +
        'exports_files(["librusty_v8.a"])\n',
    )

rusty_v8_archive = repository_rule(
    implementation = _rusty_v8_archive_impl,
    attrs = {
        "url": attr.string(mandatory = True),
        "sha256": attr.string(mandatory = True),
    },
    doc = "Downloads and decompresses one librusty_v8 static library.",
)
