//! Standard input that arrives while a command runs (PLAN §6, M6). A caller holds the sender and
//! writes chunks as the user types them. The shell reads them with `LiveInput`. A dropped sender
//! and an explicit `end` both end the input, so a reader sees end of file either way.

use tokio::sync::mpsc;

/// The reading end, held by the shell while a command runs.
#[derive(Debug)]
pub struct LiveInput {
    rx: mpsc::UnboundedReceiver<Option<Vec<u8>>>,
}

/// The writing end, held by the caller.
#[derive(Debug, Clone)]
pub struct LiveSender {
    tx: mpsc::UnboundedSender<Option<Vec<u8>>>,
}

/// A new live input and the sender that feeds it.
pub fn live_input() -> (LiveSender, LiveInput) {
    let (tx, rx) = mpsc::unbounded_channel();
    (LiveSender { tx }, LiveInput { rx })
}

impl LiveSender {
    /// Adds bytes to the input. Returns false once the reader has gone.
    pub fn send(&self, data: Vec<u8>) -> bool {
        self.tx.send(Some(data)).is_ok()
    }

    /// Ends the input: the reader sees end of file after the bytes already sent.
    pub fn end(&self) {
        let _ = self.tx.send(None);
    }
}

impl LiveInput {
    /// The next chunk, or `None` at end of input.
    pub async fn next_chunk(&mut self) -> Option<Vec<u8>> {
        self.rx.recv().await.flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
