//! Generates prost and tonic code from `proto/`.
//!
//! Requires `protoc` on PATH (or `PROTOC` set). The descriptor set is written
//! to OUT_DIR so tests can compare the schema against `sandbox-types`.

use std::path::{Path, PathBuf};

fn collect_protos(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_protos(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "proto") {
            out.push(path);
        }
    }
    Ok(())
}

/// On Windows, `canonicalize` returns a `\\?\` verbatim path. protoc cannot map
/// files under such a path to their import names, so the prefix is removed.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(plain) => PathBuf::from(plain),
        None => path,
    }
}

fn main() {
    // Bazel sets SANDBOX_PROTO_ROOT to the proto/buf.yaml path, because its sandbox
    // does not place proto/ at ../../proto from the crate. Cargo uses the relative path.
    println!("cargo:rerun-if-env-changed=SANDBOX_PROTO_ROOT");
    println!("cargo:rerun-if-env-changed=SANDBOX_PROTO_WKT");
    let candidate = match std::env::var_os("SANDBOX_PROTO_ROOT") {
        Some(value) => {
            let path = PathBuf::from(value);
            if path.is_file() {
                path.parent().map(Path::to_path_buf).unwrap_or(path)
            } else {
                path
            }
        }
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../proto"),
    };
    let proto_root = without_verbatim_prefix(
        candidate
            .canonicalize()
            .unwrap_or_else(|e| panic!("proto root {} not found: {e}", candidate.display())),
    );

    if std::env::var_os("PROTOC").is_none()
        && std::process::Command::new("protoc")
            .arg("--version")
            .output()
            .is_err()
    {
        panic!("protoc is required to build sandbox-proto; install it or set PROTOC");
    }

    // Bazel's protoc has no built-in well-known types. SANDBOX_PROTO_WKT lists
    // well-known .proto files; the root above google/protobuf is the include root.
    // The list is colon-separated (std::env::split_paths), not whitespace-separated,
    // because the workspace path can contain spaces.
    let mut includes = vec![proto_root.clone()];
    if let Some(value) = std::env::var_os("SANDBOX_PROTO_WKT") {
        for file in std::env::split_paths(&value) {
            if file.ends_with("google/protobuf/empty.proto") {
                if let Some(root) = file.parent().and_then(Path::parent).and_then(Path::parent) {
                    includes.push(root.to_path_buf());
                }
            }
        }
    }

    let mut protos = Vec::new();
    collect_protos(&proto_root.join("sandbox"), &mut protos)
        .unwrap_or_else(|e| panic!("reading proto files failed: {e}"));
    protos.sort();

    for proto in &protos {
        println!("cargo:rerun-if-changed={}", proto.display());
    }
    println!("cargo:rerun-if-changed={}", proto_root.display());

    let descriptor_path = PathBuf::from(
        std::env::var("OUT_DIR").unwrap_or_else(|e| panic!("OUT_DIR is set by cargo: {e}")),
    )
    .join("descriptor.bin");

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .file_descriptor_set_path(&descriptor_path)
        .compile_protos(&protos, &includes)
        .unwrap_or_else(|e| panic!("protobuf codegen failed: {e}"));
}
