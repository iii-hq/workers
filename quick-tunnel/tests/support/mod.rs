//! A scripted provider: records start/stop and lets the test push signals.
use std::{
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use quick_tunnel::{
    prerequisites::Prerequisites,
    provider::{Provider, Session, Signal, Sink},
};
use tokio::sync::watch;

pub struct FakeProvider {
    /// Returned by every `start` while set.
    pub start_error: Mutex<Option<String>>,
    pub stops: Arc<AtomicUsize>,
    starts: watch::Sender<usize>,
    sink: Mutex<Option<Sink>>,
}

impl Default for FakeProvider {
    fn default() -> Self {
        Self {
            start_error: Mutex::default(),
            stops: Arc::default(),
            starts: watch::channel(0).0,
            sink: Mutex::default(),
        }
    }
}

impl FakeProvider {
    pub fn starts(&self) -> usize {
        *self.starts.borrow()
    }

    pub fn stops(&self) -> usize {
        self.stops.load(Ordering::SeqCst)
    }

    /// Waits until `start` ran `count` times, so `send` reaches that generation.
    pub async fn started(&self, count: usize) {
        let mut starts = self.starts.subscribe();
        tokio::time::timeout(Duration::from_secs(5), starts.wait_for(|n| *n >= count))
            .await
            .expect("provider start")
            .unwrap();
    }

    /// Pushes a signal into the latest generation.
    pub async fn send(&self, signal: Signal) {
        let sink = self.sink.lock().unwrap().clone().expect("started");
        assert!(sink.send(signal).await, "manager stopped");
    }
}

#[async_trait::async_trait]
impl Provider for FakeProvider {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn start(
        &self,
        _tunnel_id: &str,
        _origin: &str,
        _state_dir: &Path,
        sink: Sink,
    ) -> Result<Box<dyn Session>, String> {
        let result = match self.start_error.lock().unwrap().clone() {
            Some(error) => Err(error),
            None => {
                *self.sink.lock().unwrap() = Some(sink);
                Ok(Box::new(FakeSession(self.stops.clone())) as Box<dyn Session>)
            }
        };
        self.starts.send_modify(|n| *n += 1);
        result
    }

    async fn prerequisites(&self) -> Option<Prerequisites> {
        None
    }
}

struct FakeSession(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl Session for FakeSession {
    fn exited(&mut self) -> Option<String> {
        None
    }

    async fn stop(self: Box<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
