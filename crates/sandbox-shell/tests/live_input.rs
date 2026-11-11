//! A top-level `cat` reads live input as it arrives, and reports each chunk as it comes (M6).

use sandbox_cas::MemStore;
use sandbox_shell::{live_input, OutputChunk, OutputObserver, Shell, ShellConfig};
use sandbox_types::hash::ContentHash;
use sandbox_vfs::Vfs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn shell() -> Shell {
    let fs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    Shell::new(fs, ShellConfig::default())
}

fn collecting() -> (OutputObserver, Arc<Mutex<Vec<u8>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let observer: OutputObserver = Arc::new(move |chunk: OutputChunk| {
        sink.lock().unwrap().extend(chunk.data);
    });
    (observer, seen)
}

/// Waits until `seen` holds `wanted`, or fails after a while.
async fn wait_for(seen: &Mutex<Vec<u8>>, wanted: &[u8]) {
    for _ in 0..1000 {
        if seen.lock().unwrap().as_slice() == wanted {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "the output was {:?}, not {:?}",
        String::from_utf8_lossy(&seen.lock().unwrap()),
        String::from_utf8_lossy(wanted)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cat_reports_each_line_before_the_input_ends() {
    let (observer, seen) = collecting();
    let (tx, input) = live_input();
    let mut shell = shell();
    let run = tokio::spawn(async move {
        let result = shell.run_streamed_live("cat", Some(observer), input).await;
        result
    });

    tx.send(b"hello\n".to_vec());
    wait_for(&seen, b"hello\n").await;
    assert!(
        !run.is_finished(),
        "cat is still reading, so the run is not over"
    );

    tx.send(b"world\n".to_vec());
    wait_for(&seen, b"hello\nworld\n").await;
    tx.end();

    let result = run.await.unwrap().unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, b"hello\nworld\n");
    // The observer saw each byte once: the command's end did not report the streamed bytes again.
    assert_eq!(*seen.lock().unwrap(), b"hello\nworld\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_command_that_reads_no_input_does_not_wait_for_it() {
    let (observer, seen) = collecting();
    let (_tx, input) = live_input();
    let mut shell = shell();
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        shell.run_streamed_live("echo hi", Some(observer), input),
    )
    .await
    .expect("echo finishes without input")
    .unwrap();
    assert_eq!(result.stdout, b"hi\n");
    assert_eq!(*seen.lock().unwrap(), b"hi\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cat_with_a_file_operand_does_not_read_the_live_input() {
    let (observer, seen) = collecting();
    let (tx, input) = live_input();
    let mut shell = shell();
    shell.run("printf 'file\\n' > f").await.unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        shell.run_streamed_live("cat f", Some(observer), input),
    )
    .await
    .expect("cat of a file finishes without input")
    .unwrap();
    drop(tx);
    assert_eq!(result.stdout, b"file\n");
    assert_eq!(*seen.lock().unwrap(), b"file\n");
}

/// Starts `script` over live input, and returns the run, the sender, and what it reports.
fn start(
    script: &'static str,
) -> (
    tokio::task::JoinHandle<sandbox_shell::ExecutionResult>,
    sandbox_shell::LiveSender,
    Arc<Mutex<Vec<u8>>>,
) {
    let (observer, seen) = collecting();
    let (tx, input) = live_input();
    let mut shell = shell();
    let run = tokio::spawn(async move {
        shell
            .run_streamed_live(script, Some(observer), input)
            .await
            .unwrap()
    });
    (run, tx, seen)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rev_reverses_each_line_as_it_arrives() {
    let (run, tx, seen) = start("rev");
    tx.send(b"abc\n".to_vec());
    wait_for(&seen, b"cba\n").await;
    tx.send(b"xy\n".to_vec());
    wait_for(&seen, b"cba\nyx\n").await;
    tx.end();
    assert_eq!(run.await.unwrap().stdout, b"cba\nyx\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grep_filters_each_line_as_it_arrives() {
    let (run, tx, seen) = start("grep -v x");
    tx.send(b"x1\ny\n".to_vec());
    wait_for(&seen, b"y\n").await;
    tx.send(b"zz\n".to_vec());
    wait_for(&seen, b"y\nzz\n").await;
    tx.end();
    assert_eq!(run.await.unwrap().stdout, b"y\nzz\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nl_numbers_lines_as_they_arrive() {
    let (run, tx, seen) = start("nl");
    tx.send(b"a\n\nb\n".to_vec());
    wait_for(&seen, b"     1\ta\n       \n     2\tb\n").await;
    tx.end();
    assert_eq!(
        run.await.unwrap().stdout,
        b"     1\ta\n       \n     2\tb\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn head_stops_after_its_lines_without_waiting_for_the_end() {
    let (run, tx, seen) = start("head -n 2");
    tx.send(b"a\nb\nc\n".to_vec());
    let result = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .expect("head is done once it has its lines")
        .unwrap();
    assert_eq!(result.stdout, b"a\nb\n");
    assert_eq!(*seen.lock().unwrap(), b"a\nb\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_command_that_needs_all_its_input_still_waits_for_the_end() {
    let (run, tx, _seen) = start("sort");
    tx.send(b"b\na\n".to_vec());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !run.is_finished(),
        "sort needs every line before it can print"
    );
    tx.end();
    assert_eq!(run.await.unwrap().stdout, b"a\nb\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_streamed_pipeline_passes_each_line_through_every_stage() {
    let (run, tx, seen) = start("cat | rev");
    tx.send(b"abc\n".to_vec());
    wait_for(&seen, b"cba\n").await;
    tx.send(b"xy\n".to_vec());
    wait_for(&seen, b"cba\nyx\n").await;
    tx.end();
    assert_eq!(run.await.unwrap().stdout, b"cba\nyx\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_filter_in_a_streamed_pipeline_drops_the_lines_it_rejects() {
    let (run, tx, seen) = start("grep a | rev");
    tx.send(b"ax\nb\nxa\n".to_vec());
    wait_for(&seen, b"xa\nax\n").await;
    tx.end();
    assert_eq!(run.await.unwrap().stdout, b"xa\nax\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pipeline_ending_in_sort_prints_once_the_input_ends() {
    let (run, tx, seen) = start("cat | sort");
    tx.send(b"b\na\n".to_vec());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        seen.lock().unwrap().is_empty(),
        "sort prints only after every line"
    );
    assert!(!run.is_finished());
    tx.end();
    assert_eq!(run.await.unwrap().stdout, b"a\nb\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pipeline_whose_first_stage_needs_all_input_gets_it_drained() {
    let (run, tx, _seen) = start("sort | rev");
    tx.send(b"ab\nba\n".to_vec());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!run.is_finished(), "sort waits for the end of its input");
    tx.end();
    assert_eq!(run.await.unwrap().stdout, b"ba\nab\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn head_at_the_end_of_a_streamed_pipeline_stops_the_run() {
    let (run, tx, _seen) = start("cat | head -n 1");
    tx.send(b"x\ny\n".to_vec());
    let result = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .expect("head is done after its line")
        .unwrap();
    assert_eq!(result.stdout, b"x\n");
    drop(tx);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_streamed_pipeline_reports_its_last_stage_status() {
    let (run, tx, seen) = start("cat | rev");
    tx.send(b"abc\n".to_vec());
    wait_for(&seen, b"cba\n").await;
    tx.end();
    let result = run.await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, b"cba\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_streamed_pipeline_reports_the_last_stage_status_per_line() {
    // The last line matches, so the last stage's last invocation succeeds.
    let (run, tx, seen) = start("cat | grep a");
    tx.send(b"xyz\naaa\n".to_vec());
    wait_for(&seen, b"aaa\n").await;
    tx.end();
    let result = run.await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, b"aaa\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_streamed_pipeline_reports_a_failing_last_stage() {
    // The last line does not match, so the last stage's last invocation fails.
    let (run, tx, seen) = start("cat | grep a");
    tx.send(b"aaa\nxyz\n".to_vec());
    wait_for(&seen, b"aaa\n").await;
    tx.end();
    let result = run.await.unwrap();
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.stdout, b"aaa\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_streamed_pipeline_ending_in_a_whole_input_stage_reports_that_stage() {
    let (run, tx, seen) = start("cat | sort");
    tx.send(b"b\na\n".to_vec());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        seen.lock().unwrap().is_empty(),
        "sort prints only after every line"
    );
    tx.end();
    let result = run.await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, b"a\nb\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_streamed_pipeline_reports_zero() {
    let (run, tx, _seen) = start("cat | rev");
    tx.end();
    let result = run.await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, b"");
}
