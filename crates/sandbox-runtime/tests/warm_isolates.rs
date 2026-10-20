//! Scripts run from warm isolates (PLAN.md §4.14) keep every limit the cold path
//! keeps, and no state carries from one run to the next.

use sandbox_runtime::{execute_script, ScriptLimits};
use std::time::Duration;

#[tokio::test]
async fn a_script_that_never_ends_is_stopped_by_the_timeout() {
    let limits = ScriptLimits {
        timeout: Duration::from_millis(200),
        ..ScriptLimits::default()
    };
    let out = execute_script("while (true) {}", limits).await.unwrap();
    assert_eq!(out.exit_code, 1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("timed out"), "{stderr}");
}

#[tokio::test]
async fn output_past_the_cap_is_cut_and_marked() {
    let limits = ScriptLimits {
        max_output_bytes: 16,
        ..ScriptLimits::default()
    };
    let out = execute_script(
        "for (let i = 0; i < 100; i++) console.log('line ' + i);",
        limits,
    )
    .await
    .unwrap();
    assert!(out.stdout.len() <= 16, "{} bytes", out.stdout.len());
    assert!(String::from_utf8_lossy(&out.stderr).contains("output truncated"));
}

#[tokio::test]
async fn running_out_of_heap_is_a_script_error_not_a_crash() {
    let out = execute_script(
        "const keep = []; while (true) { keep.push(new Array(1000000).fill(1)); }",
        ScriptLimits::default(),
    )
    .await
    .unwrap();
    assert_eq!(out.exit_code, 1);
    assert!(String::from_utf8_lossy(&out.stderr).contains("heap limit"));
}

#[tokio::test]
async fn one_run_cannot_see_another_runs_globals() {
    // Each run gets its own isolate, warm or not. A global set in one run is absent in the next.
    let first = execute_script(
        "globalThis.leak = 42; console.log('set');",
        ScriptLimits::default(),
    )
    .await
    .unwrap();
    assert_eq!(first.exit_code, 0);
    let second = execute_script("console.log(typeof leak);", ScriptLimits::default())
        .await
        .unwrap();
    assert_eq!(second.exit_code, 0);
    assert_eq!(second.stdout, b"undefined\n");
}

#[tokio::test]
async fn many_runs_in_a_row_all_produce_their_own_output() {
    for i in 0..20 {
        let out = execute_script(&format!("console.log({i} * 2);"), ScriptLimits::default())
            .await
            .unwrap();
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout, format!("{}\n", i * 2).into_bytes());
    }
}
