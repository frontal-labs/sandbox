use sandbox_shell::live_input;

#[tokio::test]
async fn chunks_arrive_in_order_and_end_is_end_of_file() {
    let (tx, mut rx) = live_input();
    assert!(tx.send(b"a".to_vec()));
    assert!(tx.send(b"b".to_vec()));
    tx.end();
    assert_eq!(rx.next_chunk().await, Some(b"a".to_vec()));
    assert_eq!(rx.next_chunk().await, Some(b"b".to_vec()));
    assert_eq!(rx.next_chunk().await, None);
}

#[tokio::test]
async fn a_dropped_sender_is_end_of_file_too() {
    let (tx, mut rx) = live_input();
    drop(tx);
    assert_eq!(rx.next_chunk().await, None);
}

#[tokio::test]
async fn sending_after_the_reader_is_gone_reports_it() {
    let (tx, rx) = live_input();
    drop(rx);
    assert!(!tx.send(b"x".to_vec()));
}
