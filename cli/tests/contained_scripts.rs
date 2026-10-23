//! `node` scripts run in a contained child process. These tests drive the real child
//! (the `frontal-sandbox` binary): the host survives scripts that exhaust memory, and
//! each limit is reported. The containment checks need Linux with delegated cgroups
//! v2. Elsewhere the tests check that `node` is refused.

#[cfg(target_os = "linux")]
mod linux {
    use sandbox_runtime::{
        execute_script_contained, install_script_runner, ScriptLimits, ScriptOutcome,
    };
    use sandbox_worker::ScriptProcess;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn install() {
        let runner = ScriptProcess::new(env!("CARGO_BIN_EXE_frontal-sandbox"));
        let _ = install_script_runner(Arc::new(runner));
    }

    fn check_contained_test_enabled() {
        if std::env::var("SANDBOX_TEST_CONTAINED").is_err() {
            eprintln!("Skipping contained test: set SANDBOX_TEST_CONTAINED=1 to run (requires delegated cgroup v2)");
            panic!("test skipped: SANDBOX_TEST_CONTAINED not set");
        }
    }

    async fn run(code: &str, limits: ScriptLimits) -> ScriptOutcome {
        check_contained_test_enabled();
        install();
        execute_script_contained(code, limits, None)
            .await
            .expect("the contained runner should run the script")
    }

    fn stderr_text(out: &ScriptOutcome) -> String {
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    #[tokio::test]
    async fn output_comes_back_in_order_with_exit_zero() {
        let out = run(
            "console.log('one'); console.log('two');",
            ScriptLimits::default(),
        )
        .await;
        assert_eq!(out.exit_code, 0, "stderr: {}", stderr_text(&out));
        assert_eq!(String::from_utf8_lossy(&out.stdout), "one\ntwo\n");
    }

    #[tokio::test]
    async fn stdout_is_streamed_to_the_observer_as_it_is_printed() {
        install();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let observer: sandbox_runtime::StreamObserver =
            Arc::new(move |piece: &[u8]| sink.lock().unwrap().extend_from_slice(piece));
        let out = execute_script_contained(
            "console.log('streamed');",
            ScriptLimits::default(),
            Some(observer),
        )
        .await
        .expect("runner");
        assert_eq!(out.exit_code, 0);
        assert_eq!(*seen.lock().unwrap(), b"streamed\n");
    }

    #[tokio::test]
    async fn a_loop_that_grows_the_heap_hits_the_heap_limit() {
        let out = run(
            "const keep = []; while (true) { keep.push(new Array(1000000).fill(1)); }",
            ScriptLimits::default(),
        )
        .await;
        assert_eq!(out.exit_code, 1);
        assert!(
            stderr_text(&out).contains("heap limit"),
            "{}",
            stderr_text(&out)
        );
    }

    #[tokio::test]
    async fn a_single_huge_allocation_kills_only_the_script_process() {
        // A fatal V8 out-of-memory aborts the child. The host must survive it and
        // report the script as failed.
        let out = run(
            "const a = new Array(200000000).fill(1);",
            ScriptLimits::default(),
        )
        .await;
        assert_eq!(out.exit_code, 1);
        assert!(!stderr_text(&out).is_empty());
    }

    #[tokio::test]
    async fn memory_outside_the_v8_heap_is_bounded_by_the_cgroup() {
        // Typed arrays are allocated outside the V8 heap, so the heap limit does not
        // count them. The cgroup limit does.
        let out = run(
            // fill() makes the pages resident. Untouched zero-filled buffers cost nothing.
            "const keep = []; while (true) { keep.push(new Uint8Array(1 << 26).fill(1)); }",
            ScriptLimits::default(),
        )
        .await;
        assert_eq!(
            out.exit_code,
            1,
            "stdout: {:?}, stderr: {}",
            out.stdout.len(),
            stderr_text(&out)
        );
        assert!(
            stderr_text(&out).contains("memory limit"),
            "{}",
            stderr_text(&out)
        );
    }

    #[tokio::test]
    async fn a_string_larger_than_the_budget_is_bounded_by_the_cgroup() {
        // The budget is the heap limit plus the fixed overhead. 256 MiB is past it.
        let limits = ScriptLimits {
            max_heap_bytes: 32 << 20,
            ..ScriptLimits::default()
        };
        // repeat() builds a rope, so .length alone allocates nothing. indexOf flattens
        // the string, which makes the memory resident.
        let out = run("const s = 'x'.repeat(256 << 20); s.indexOf('y');", limits).await;
        assert_eq!(
            out.exit_code,
            1,
            "stdout: {:?}, stderr: {}",
            String::from_utf8_lossy(&out.stdout),
            stderr_text(&out)
        );
        let stderr = stderr_text(&out);
        assert!(
            stderr.contains("memory limit") || stderr.contains("heap limit"),
            "{stderr}"
        );
    }

    #[tokio::test]
    async fn an_endless_script_times_out() {
        let limits = ScriptLimits {
            timeout: Duration::from_millis(300),
            ..ScriptLimits::default()
        };
        let out = run("while (true) {}", limits).await;
        assert_eq!(out.exit_code, 1);
        assert!(
            stderr_text(&out).contains("timed out"),
            "{}",
            stderr_text(&out)
        );
    }

    #[tokio::test]
    async fn stdout_past_the_cap_is_cut_and_marked() {
        let limits = ScriptLimits {
            max_output_bytes: 16,
            ..ScriptLimits::default()
        };
        let out = run("console.log('x'.repeat(1000));", limits).await;
        assert_eq!(out.stdout.len(), 16);
        assert!(stderr_text(&out).contains("output truncated"));
    }
}

#[cfg(not(target_os = "linux"))]
mod elsewhere {
    use sandbox_runtime::{execute_script_contained, install_script_runner, ScriptLimits};
    use sandbox_types::ErrorCode;
    use sandbox_worker::ScriptProcess;
    use std::sync::Arc;

    #[tokio::test]
    async fn node_is_refused_without_linux_containment() {
        let _ = install_script_runner(Arc::new(ScriptProcess::new(env!(
            "CARGO_BIN_EXE_frontal-sandbox"
        ))));
        let err = execute_script_contained("console.log(1)", ScriptLimits::default(), None)
            .await
            .expect_err("contained scripts need Linux");
        assert_eq!(err.code, ErrorCode::Unsupported);
    }
}
