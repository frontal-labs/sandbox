use sandbox_types::Event;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct TraceLog {
    events: Arc<Mutex<VecDeque<Event>>>,
}

impl TraceLog {
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub async fn append(&self, event: Event) {
        self.events.lock().await.push_back(event);
    }

    pub async fn events(&self) -> Vec<Event> {
        self.events.lock().await.iter().cloned().collect()
    }
}

impl Default for TraceLog {
    fn default() -> Self {
        Self::new()
    }
}
