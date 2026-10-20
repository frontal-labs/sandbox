#![no_main]

use libfuzzer_sys::fuzz_target;
use sandbox_cas::MemStore;
use sandbox_shell::{Shell, ShellConfig};
use sandbox_types::hash::ContentHash;
use sandbox_vfs::Vfs;
use std::sync::{Arc, OnceLock};

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("fuzz runtime")
    })
}

// Any script must finish with an error or an exit code, never panic. The step
// and output budgets keep every run short.
fuzz_target!(|data: &[u8]| {
    let Ok(script) = std::str::from_utf8(data) else {
        return;
    };
    let config = ShellConfig {
        max_steps: 10_000,
        max_output_bytes: 64 * 1024,
        ..ShellConfig::default()
    };
    runtime().block_on(async {
        let fs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
        let mut shell = Shell::new(fs, config);
        let _ = shell.run(script).await;
    });
});
