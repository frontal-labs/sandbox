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
