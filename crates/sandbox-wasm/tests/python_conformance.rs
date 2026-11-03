//! CPython 3.13.1 on the sandbox's WASI host, against the output of the same interpreter
//! built natively (`tests/data/python_cases.json`, recorded with the dev image's native build).
//!
//! Ignored by default, because it needs the built artifacts from `tools/wasm/python/build.sh`:
//!   SANDBOX_PYTHON_WASM=.../python.wasm SANDBOX_PYTHON_ZIP=.../python313.zip \
//!     cargo test -p sandbox-wasm --test python_conformance -- --ignored

use sandbox_cas::MemStore;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::{OpenFlags, SessionFs, Vfs};
use sandbox_wasm::{run_module_with_preopens_streamed, Mount, Preopen, WasiLimits};
use std::path::Path;
use std::sync::Arc;

/// Fuel for one CPython run. Startup and a json import take 1e9 to 3e9 instructions.
const PYTHON_FUEL: u64 = 5_000_000_000;
/// Memory for one CPython run. The default 64 MiB is enough for small scripts, and this
/// leaves room for real work.
const PYTHON_MEMORY: usize = 256 * 1024 * 1024;

#[derive(serde::Deserialize)]
struct Case {
    name: String,
    code: String,
    stdout: String,
    exit_code: i32,
}

/// A session's VFS: a work directory, and the stdlib zip under `/usr/lib`, as the
/// session's read-only system tree.
async fn session(zip: &[u8]) -> Arc<Vfs> {
    let vfs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    vfs.mkdir(Path::new("/usr"), 0o755).await.unwrap();
    vfs.mkdir(Path::new("/usr/lib"), 0o755).await.unwrap();
    let flags = OpenFlags {
        write: true,
        create: true,
        ..OpenFlags::default()
    };
    let handle = vfs
        .open(Path::new("/usr/lib/python313.zip"), flags)
        .await
        .unwrap();
    vfs.write(&handle, 0, zip).await.unwrap();
    vfs.close(&handle);
    vfs
}

async fn run(wasm: &[u8], zip: &[u8], code: &str) -> (i32, String, String) {
    let vfs = session(zip).await;
    let limits = WasiLimits {
        fuel: PYTHON_FUEL,
        max_memory_bytes: PYTHON_MEMORY,
        ..WasiLimits::default()
    };
    let out = run_module_with_preopens_streamed(
        wasm.to_vec(),
        vec!["/usr/bin/python".into(), "-c".into(), code.into()],
        Vec::new(),
        limits,
        Some(Mount {
            vfs,
            root: "/work".to_string(),
        }),
        vec![Preopen {
            guest_path: "/usr".to_string(),
            root: "/usr".to_string(),
            readonly: true,
        }],
        None,
    )
    .await
    .expect("the host runs CPython");
    (
        out.exit_code,
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[tokio::test]
#[ignore = "needs tools/wasm/python/out; see the module comment"]
async fn cpython_matches_the_native_build() {
    let wasm =
        std::fs::read(std::env::var("SANDBOX_PYTHON_WASM").expect("set SANDBOX_PYTHON_WASM"))
            .unwrap();
    let zip = std::fs::read(std::env::var("SANDBOX_PYTHON_ZIP").expect("set SANDBOX_PYTHON_ZIP"))
        .unwrap();
    let cases: Vec<Case> = serde_json::from_str(include_str!("data/python_cases.json")).unwrap();
    let mut failures = Vec::new();
    for case in &cases {
        let (exit_code, stdout, stderr) = run(&wasm, &zip, &case.code).await;
        if exit_code != case.exit_code || stdout != case.stdout {
            failures.push(format!(
                "{}: exit {} (want {}), stdout {:?} (want {:?}), stderr {:?}",
                case.name,
                exit_code,
                case.exit_code,
                stdout,
                case.stdout,
                stderr.chars().take(300).collect::<String>()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}
