//! Streamed output: chunks arrive per finished top-level command, output the shell
//! consumes is not streamed, and the chunks always add up to the final result.

use sandbox_cas::MemStore;
use sandbox_shell::{ExecutionResult, OutputChunk, OutputObserver, Shell, ShellConfig};
use sandbox_types::hash::ContentHash;
use sandbox_vfs::Vfs;
use std::sync::{Arc, Mutex};

fn shell_with(config: ShellConfig) -> Shell {
    let fs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    Shell::new(fs, config)
}

/// Runs `script` and returns the chunks in the order they were reported, and the result.
async fn streamed(script: &str) -> (Vec<OutputChunk>, ExecutionResult) {
    streamed_with(ShellConfig::default(), script).await
}

async fn streamed_with(config: ShellConfig, script: &str) -> (Vec<OutputChunk>, ExecutionResult) {
    let chunks = Arc::new(Mutex::new(Vec::new()));
    let sink = chunks.clone();
    let observer: OutputObserver = Arc::new(move |chunk| sink.lock().unwrap().push(chunk));
    let mut shell = shell_with(config);
    let result = shell
        .run_streamed(script, Some(observer))
        .await
        .expect("script parses");
    let chunks = chunks.lock().unwrap().clone();
    (chunks, result)
}

fn text(chunks: &[OutputChunk], fd: u8) -> Vec<String> {
    chunks
        .iter()
        .filter(|c| c.fd == fd)
        .map(|c| String::from_utf8_lossy(&c.data).into_owned())
        .collect()
}

fn joined(chunks: &[OutputChunk], fd: u8) -> Vec<u8> {
    chunks
        .iter()
        .filter(|c| c.fd == fd)
        .flat_map(|c| c.data.clone())
        .collect()
}

#[tokio::test]
async fn each_finished_command_is_one_chunk_in_order() {
    let (chunks, _) = streamed("echo a; echo b").await;
    assert_eq!(text(&chunks, 1), ["a\n", "b\n"]);
}

#[tokio::test]
async fn a_loop_streams_each_iteration() {
    let (chunks, _) = streamed("for i in 1 2 3; do echo $i; done").await;
    assert_eq!(text(&chunks, 1), ["1\n", "2\n", "3\n"]);
}

#[tokio::test]
async fn a_function_call_is_one_command_reported_once() {
    // The body's output is reported when the call finishes, and not again per command.
    let (chunks, _) = streamed("f() { echo in-f; echo more; }; f").await;
    assert_eq!(text(&chunks, 1), ["in-f\nmore\n"]);
}

#[tokio::test]
async fn a_subshell_streams_its_commands() {
    let (chunks, _) = streamed("(echo a; echo b)").await;
    assert_eq!(text(&chunks, 1), ["a\n", "b\n"]);
}

#[tokio::test]
async fn only_the_last_stage_of_a_pipeline_streams() {
    // `echo` feeds `cat`, so the pipe's output is `cat`'s alone.
    let (chunks, result) = streamed("echo a | cat").await;
    assert_eq!(text(&chunks, 1), ["a\n"]);
    assert_eq!(result.stdout, b"a\n");
}

#[tokio::test]
async fn output_redirected_to_a_file_is_not_streamed() {
    let (chunks, _) = streamed("echo a > f; cat f").await;
    assert_eq!(
        text(&chunks, 1),
        ["a\n"],
        "only `cat` writes to the top level"
    );
}

#[tokio::test]
async fn a_redirected_compound_is_streamed_once_as_routed() {
    let (chunks, _) = streamed("{ echo a; echo b; } > f; cat f").await;
    assert_eq!(text(&chunks, 1), ["a\nb\n"]);
}

#[tokio::test]
async fn a_substitution_is_not_streamed_but_its_word_is() {
    let (chunks, _) = streamed("echo $(echo inner)").await;
    assert_eq!(text(&chunks, 1), ["inner\n"]);
}

#[tokio::test]
async fn stderr_is_streamed_on_its_own_channel() {
    let (chunks, result) = streamed("cat nope; echo ok").await;
    assert!(text(&chunks, 2).concat().contains("nope"), "{chunks:?}");
    assert_eq!(text(&chunks, 1), ["ok\n"]);
    assert!(!result.stderr.is_empty());
}

#[tokio::test]
async fn output_before_an_abort_is_still_streamed() {
    // The step limit stops the loop partway. The iterations that ran were already sent.
    let config = ShellConfig {
        max_steps: 4,
        ..ShellConfig::default()
    };
    let (chunks, result) =
        streamed_with(config, "for i in 1 2 3 4 5 6 7 8; do echo $i; done").await;
    let streamed = text(&chunks, 1);
    assert!(!streamed.is_empty(), "no chunk before the abort");
    assert_eq!(result.exit_code, 1);
    assert_eq!(joined(&chunks, 1), result.stdout);
}

#[tokio::test]
async fn the_chunks_add_up_to_the_result() {
    let scripts = [
        "echo a; echo b >&2; echo c",
        "for i in 1 2; do echo $i; done | cat; echo end",
        "f() { echo in-f; return 3; }; f; echo $?",
        "x=$(echo sub); echo \"got $x\"",
        "case x in x) echo yes;; esac; echo done",
        "(echo p; echo q) | cat; echo r",
        "{ echo g; echo h >&2; } 2>&1 | cat",
        "cat nope; echo after",
        "echo first > out; cat out out",
        "node -e \"console.log(1); console.log(2)\"; echo end",
        "node -e \"console.log('x'); throw new Error('boom')\"",
    ];
    for script in scripts {
        let (chunks, result) = streamed(script).await;
        assert_eq!(joined(&chunks, 1), result.stdout, "stdout of {script:?}");
        assert_eq!(joined(&chunks, 2), result.stderr, "stderr of {script:?}");
    }
}

#[tokio::test]
async fn run_without_an_observer_reports_nothing_new() {
    // Plain `run` keeps its result the same, with no observer set.
    let mut shell = shell_with(ShellConfig::default());
    let out = shell.run("echo a; echo b").await.unwrap();
    assert_eq!(out.stdout, b"a\nb\n");
}

#[tokio::test]
async fn an_argv_command_is_one_chunk_per_stream() {
    let chunks = Arc::new(Mutex::new(Vec::new()));
    let sink = chunks.clone();
    let observer: OutputObserver = Arc::new(move |chunk| sink.lock().unwrap().push(chunk));
    let mut shell = shell_with(ShellConfig::default());
    let argv = ["cat".to_string()];
    let out = shell
        .run_argv_streamed(&argv, b"piped\n", Some(observer))
        .await
        .unwrap();
    let chunks = chunks.lock().unwrap().clone();
    assert_eq!(text(&chunks, 1), ["piped\n"]);
    assert_eq!(out.stdout, b"piped\n");
}

#[tokio::test]
async fn a_node_script_streams_each_line_as_it_is_printed() {
    let _ = sandbox_runtime::install_in_process_script_runner();
    // Each `console.log` is one chunk, sent while the script runs. The end of the
    // command is not waited for.
    let (chunks, result) =
        streamed("node -e \"console.log('one'); console.log('two'); console.log('three')\"").await;
    assert_eq!(text(&chunks, 1), ["one\n", "two\n", "three\n"]);
    assert_eq!(result.stdout, b"one\ntwo\nthree\n");
}

#[tokio::test]
async fn a_node_error_after_output_keeps_the_output_streamed() {
    let _ = sandbox_runtime::install_in_process_script_runner();
    let (chunks, result) =
        streamed("node -e \"console.log('before'); throw new Error('boom')\"").await;
    assert_eq!(text(&chunks, 1), ["before\n"]);
    assert_eq!(result.exit_code, 1);
    assert!(String::from_utf8_lossy(&result.stderr).contains("boom"));
    assert_eq!(joined(&chunks, 2), result.stderr);
}

#[tokio::test]
async fn node_output_redirected_to_a_file_is_not_streamed() {
    let _ = sandbox_runtime::install_in_process_script_runner();
    let (chunks, _) = streamed("node -e \"console.log('kept')\" > out; cat out").await;
    assert_eq!(
        text(&chunks, 1),
        ["kept\n"],
        "only `cat` writes to the top level"
    );
}

#[tokio::test]
async fn node_output_inside_a_pipe_is_not_streamed() {
    let _ = sandbox_runtime::install_in_process_script_runner();
    let (chunks, result) = streamed("node -e \"console.log('a'); console.log('b')\" | cat").await;
    assert_eq!(
        text(&chunks, 1),
        ["a\nb\n"],
        "one chunk, from `cat`, the last stage"
    );
    assert_eq!(result.stdout, b"a\nb\n");
}
