//! Recursion in a script is bounded by a command depth limit, so a recursive
//! function returns an error and does not overflow the stack (ADR 0018). A stack
//! overflow aborts the whole process, and the isolate backend runs in the server
//! process, so each case runs on a thread with the 2 MiB stack that a tokio
//! worker gets by default.

use sandbox_cas::MemStore;
use sandbox_shell::{ExecutionResult, Shell, ShellConfig};
use sandbox_types::hash::ContentHash;
use sandbox_types::ErrorCode;
use sandbox_vfs::Vfs;
use std::sync::Arc;

const WORKER_STACK: usize = 2 * 1024 * 1024;

fn run_on_worker_stack(script: impl Into<String>) -> ExecutionResult {
    let script: String = script.into();
    std::thread::Builder::new()
        .stack_size(WORKER_STACK)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let fs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
                let mut shell = Shell::new(fs, ShellConfig::default());
                shell.run(&script).await.unwrap()
            })
        })
        .unwrap()
        .join()
        .expect("the shell thread panicked")
}

fn assert_stopped_by_depth_limit(script: &str) {
    let out = run_on_worker_stack(script);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.exit_code, 1, "{script}: {stderr}");
    assert!(stderr.contains("nested deeper than"), "{script}: {stderr}");
}

#[test]
fn a_function_that_calls_itself_is_stopped() {
    assert_stopped_by_depth_limit("f() { f; }; f");
}

#[test]
fn the_input_that_overflowed_the_stack_under_fuzzing_is_stopped() {
    // The 10-byte input fuzz/shell_run found: a function whose body calls
    // itself, after a NUL byte.
    assert_stopped_by_depth_limit("\0\nH()H\n\nH\n");
}

#[test]
fn recursion_through_an_if_is_stopped() {
    assert_stopped_by_depth_limit("f() { if true; then f; fi; }; f");
}

#[test]
fn recursion_through_a_subshell_is_stopped() {
    assert_stopped_by_depth_limit("f() { ( f ); }; f");
}

#[test]
fn recursion_through_a_substitution_is_stopped() {
    assert_stopped_by_depth_limit("f() { echo $(f); }; f");
}

#[test]
fn recursion_through_a_loop_is_stopped() {
    assert_stopped_by_depth_limit("f() { while true; do f; done; }; f");
}

#[test]
fn recursion_through_a_group_is_stopped() {
    assert_stopped_by_depth_limit("f() { { f; }; }; f");
}

#[test]
fn nesting_within_the_limit_runs() {
    let script = format!(
        "{}echo ok{}",
        "if true; then ".repeat(10),
        "; fi".repeat(10)
    );
    let out = run_on_worker_stack(script);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, b"ok\n");
}

#[test]
fn deep_compound_nesting_is_refused_by_the_parser() {
    let deep = "if true; then ".repeat(5_000);
    let err = std::thread::Builder::new()
        .stack_size(WORKER_STACK)
        .spawn(move || sandbox_shell::parse(&deep).unwrap_err())
        .unwrap()
        .join()
        .expect("the parser thread panicked");
    assert_eq!(err.code, ErrorCode::ResourceExhausted);
}
