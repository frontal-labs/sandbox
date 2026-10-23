//! Scripts that try to exhaust memory must fail as script errors. The host process must
//! survive, and the script must not keep growing memory past its limit.

use sandbox_runtime::{execute_script, ScriptLimits};

async fn assert_contained(code: &str) {
    let out = execute_script(code, ScriptLimits::default())
        .await
        .expect("host error");
    assert_eq!(
        out.exit_code,
        1,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
async fn growing_arrays_in_a_loop_is_contained() {
    assert_contained("const keep = []; while (true) { keep.push(new Array(1000000).fill(1)); }")
        .await;
}

#[ignore = "in-process only: a fatal V8 OOM aborts this process. Node runs contained; see cli/tests/contained_scripts.rs"]
#[tokio::test]
async fn one_huge_allocation_is_contained() {
    assert_contained("const a = new Array(200000000).fill(1);").await;
}

#[tokio::test]
async fn doubling_strings_is_contained() {
    assert_contained("let s = 'x'; while (true) { s += s; }").await;
}

#[tokio::test]
async fn typed_arrays_outside_the_v8_heap_are_contained() {
    assert_contained("const keep = []; while (true) { keep.push(new Uint8Array(1 << 26)); }").await;
}

#[ignore = "in-process only: string growth is not bounded by max_heap_bytes. Node runs contained; see cli/tests/contained_scripts.rs"]
#[tokio::test]
async fn one_string_bigger_than_the_limit_is_contained() {
    let limits = ScriptLimits {
        max_heap_bytes: 32 << 20,
        ..ScriptLimits::default()
    };
    let out = execute_script("const s = 'x'.repeat(64 << 20); s.length;", limits)
        .await
        .expect("host error");
    assert_eq!(out.exit_code, 1);
}
