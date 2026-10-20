//! Glob matching is polynomial in the pattern and the name. The naive matcher was
//! exponential in the number of `*`, and a 155-byte script ran for over a minute
//! under `fuzz/shell_run`. The step budget does not count work inside one word, so
//! the matcher itself must be bounded.

use sandbox_cas::MemStore;
use sandbox_shell::{Shell, ShellConfig};
use sandbox_types::hash::ContentHash;
use sandbox_vfs::Vfs;
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// Runs `script` on a worker thread and fails if it takes longer than `limit`.
fn finishes_within(script: String, limit: Duration) -> bool {
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let fs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
            let mut shell = Shell::new(fs, ShellConfig::default());
            let _ = shell.run(&script).await;
        });
        let _ = done.send(());
    });
    finished.recv_timeout(limit).is_ok()
}

#[test]
fn the_input_that_ran_for_over_a_minute_under_fuzzing_finishes() {
    assert!(finishes_within(
        "\x01\0\0>>?>\x01>\x01\x0e>>\x01>\x01$>\x0e\x01>?\x01>*>?>$2*******************$>\x0e>???**************\x0a**************j**********************************************************>??******$?".to_string(),
        Duration::from_secs(10)
    ));
}

#[test]
fn a_pattern_with_many_stars_is_matched_in_polynomial_time() {
    // The glob is matched against the names that exist, so one is created first. A
    // name of 60 `a`s against 40 `*a` pieces and a final `b` is about 2^40 steps
    // for the naive matcher.
    let script = format!("touch {}; echo {}b", "a".repeat(60), "*a".repeat(40));
    assert!(finishes_within(script, Duration::from_secs(10)));
}
