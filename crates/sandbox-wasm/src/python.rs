//! Installs CPython into a session's filesystem: the WASI module at `/opt/python.wasm`, its
//! stdlib at `/usr/lib/python313.zip`, and a `python` entry in the tool manifest at
//! `/etc/tools.toml`. The artifacts come from `tools/wasm/python/build.sh`. The install is a
//! host write, so the agent sees the files as ordinary files, and the content store keeps one
//! copy of each blob across sessions, because the bytes are the same.
//!
//! The entry runs CPython with argv[0] `/usr/bin/python`, which is how it finds its home: the
//! guest has no environment. The stdlib is mounted read-only at `/usr`, so the session cannot
//! change it.

use crate::registry::Registry;
use sandbox_types::{ErrorCode, SandboxError};
use sandbox_vfs::{OpenFlags, SessionFs};
use std::path::{Path, PathBuf};

/// Where the module is installed, and the path the manifest names.
pub const PYTHON_MODULE_PATH: &str = "/opt/python.wasm";
/// Where the stdlib zip is installed. CPython finds it at `lib/python313.zip` under `/usr`.
pub const PYTHON_STDLIB_PATH: &str = "/usr/lib/python313.zip";
/// Fuel for one run. Startup and a json import take 1e9 to 3e9 instructions (docs/PYTHON_WASM.md).
pub const PYTHON_FUEL: u64 = 5_000_000_000;
/// Memory for one run.
pub const PYTHON_MEMORY: usize = 256 * 1024 * 1024;

/// The CPython artifacts, read from a build directory.
#[derive(Debug, Clone)]
pub struct PythonArtifacts {
    pub module: Vec<u8>,
    pub stdlib: Vec<u8>,
}

impl PythonArtifacts {
    /// Reads `python.wasm` and `python313.zip` from `dir`.
    pub fn load(dir: &Path) -> Result<Self, SandboxError> {
        let read = |name: &str| -> Result<Vec<u8>, SandboxError> {
            let path: PathBuf = dir.join(name);
            std::fs::read(&path).map_err(|e| {
                SandboxError::new(
                    ErrorCode::InvalidArgument,
                    format!("cannot read {}: {e}", path.display()),
                )
            })
        };
        Ok(PythonArtifacts {
            module: read("python.wasm")?,
            stdlib: read("python313.zip")?,
        })
    }
}

/// The tool manifest. The shell reads the same path (`sandbox_shell::interpreter::TOOLS_MANIFEST`),
/// and this crate cannot name that constant, because the shell depends on this crate.
pub const MANIFEST_PATH: &str = "/etc/tools.toml";

/// The manifest entry for `python`.
pub fn registry_entry() -> String {
    format!(
        "[[tool]]\nname = \"python\"\npath = \"{PYTHON_MODULE_PATH}\"\nversion = \"3.13.1\"\n\
         argv0 = \"/usr/bin/python\"\npreopen = [\"/usr\"]\nfuel = {PYTHON_FUEL}\n\
         max_memory_bytes = {PYTHON_MEMORY}\n"
    )
}

/// Writes the module, the stdlib, and the manifest entry into `fs`. A manifest that already
/// names `python` is left as it is, and any other tools in it are kept.
pub async fn install(fs: &dyn SessionFs, artifacts: &PythonArtifacts) -> Result<(), SandboxError> {
    for dir in ["/opt", "/usr", "/usr/lib", "/etc"] {
        ensure_dir(fs, dir).await?;
    }
    write_file(fs, PYTHON_MODULE_PATH, &artifacts.module).await?;
    write_file(fs, PYTHON_STDLIB_PATH, &artifacts.stdlib).await?;

    let existing = read_manifest(fs).await?;
    let manifest = match existing {
        Some(text) => {
            let registry = Registry::parse(&text)?;
            if registry.find("python").is_some() {
                text
            } else {
                format!("{}\n{}", text.trim_end(), registry_entry())
            }
        }
        None => registry_entry(),
    };
    write_file(fs, MANIFEST_PATH, manifest.as_bytes()).await
}

async fn ensure_dir(fs: &dyn SessionFs, path: &str) -> Result<(), SandboxError> {
    match fs.stat(Path::new(path)).await {
        Ok(stat) if stat.is_dir => Ok(()),
        _ => fs.mkdir(Path::new(path), 0o755).await.map_err(|e| {
            SandboxError::new(ErrorCode::VfsError, format!("cannot create {path}: {e}"))
        }),
    }
}

async fn read_manifest(fs: &dyn SessionFs) -> Result<Option<String>, SandboxError> {
    let path = MANIFEST_PATH;
    let Ok(stat) = fs.stat(Path::new(path)).await else {
        return Ok(None);
    };
    let handle = fs
        .open(Path::new(path), OpenFlags::default())
        .await
        .map_err(|e| SandboxError::new(ErrorCode::VfsError, e.to_string()))?;
    let bytes = fs
        .read(&handle, 0, stat.size as usize)
        .await
        .map_err(|e| SandboxError::new(ErrorCode::VfsError, e.to_string()));
    fs.close(&handle);
    let text = String::from_utf8(bytes?.to_vec()).map_err(|_| {
        SandboxError::new(ErrorCode::InvalidArgument, "the tool manifest is not UTF-8")
    })?;
    Ok(Some(text))
}

/// Writes a whole file, in chunks, replacing what was there.
async fn write_file(fs: &dyn SessionFs, path: &str, data: &[u8]) -> Result<(), SandboxError> {
    const CHUNK: usize = 1024 * 1024;
    let flags = OpenFlags {
        read: false,
        write: true,
        create: true,
        truncate: true,
        ..OpenFlags::default()
    };
    let handle = fs
        .open(Path::new(path), flags)
        .await
        .map_err(|e| SandboxError::new(ErrorCode::VfsError, format!("cannot write {path}: {e}")))?;
    let mut at = 0usize;
    let mut first = true;
    let mut result = Ok(());
    while result.is_ok() && (first || at < data.len()) {
        first = false;
        let end = (at + CHUNK).min(data.len());
        result = fs
            .write(&handle, at as u64, &data[at..end])
            .await
            .map(|_| ())
            .map_err(|e| {
                SandboxError::new(ErrorCode::VfsError, format!("cannot write {path}: {e}"))
            });
        at = end;
    }
    fs.close(&handle);
    result
}
