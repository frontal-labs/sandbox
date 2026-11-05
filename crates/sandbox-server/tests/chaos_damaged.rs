//! A complete record that does not parse is damage, not a crash: the store refuses to open,
//! rather than start from a state with a hole in it.

use sandbox_scheduler::Scheduler;
use sandbox_server::open_durable;
use std::path::PathBuf;
use std::sync::Arc;

fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sandbox-chaos-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[tokio::test]
async fn a_damaged_complete_record_refuses_to_open() {
    let dir = fresh_dir("damaged");
    let (_app, new_key) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    assert!(new_key.is_some());
    drop(_app);

    let log = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.file_name().unwrap().to_string_lossy().ends_with(".log"))
        .unwrap_or_else(|| {
            // The first run writes its key to the log. Put a damaged complete record in place.
            let path = dir.join("state.0.log");
            std::fs::write(&path, b"").unwrap();
            path
        });
    let damaged = b"{\"op\": \"put_key\", \"value\": not json}\n";
    let mut content = std::fs::read(&log).unwrap();
    content.extend_from_slice(damaged);
    std::fs::write(&log, content).unwrap();

    match open_durable(Arc::new(Scheduler::new()), &dir).await {
        Ok(_) => panic!("a damaged record must refuse to open"),
        Err(error) => assert!(error.message.contains("damaged"), "{}", error.message),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
