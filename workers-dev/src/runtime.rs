use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{broadcast, RwLock};

use crate::config::LOG_RING_CAPACITY;

#[derive(Debug, Clone)]
pub struct RingBuffer {
    lines: VecDeque<String>,
    capacity: usize,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(capacity.min(64)),
            capacity,
        }
    }

    pub fn push(&mut self, line: String) {
        if self.lines.len() >= self.capacity {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    pub fn tail(&self, n: usize) -> Vec<String> {
        let start = self.lines.len().saturating_sub(n);
        self.lines.iter().skip(start).cloned().collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcState {
    Stopped,
    Compiling,
    Running,
    Crashed,
}

impl ProcState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Compiling => "compiling",
            Self::Running => "running",
            Self::Crashed => "crashed",
        }
    }
}

#[derive(Debug)]
pub struct WorkerRuntime {
    pub proc_state: ProcState,
    pub exit_code: Option<i32>,
    pub started_at: Option<Instant>,
    pub logs: RingBuffer,
    pub log_tx: broadcast::Sender<String>,
    /// Injectable-UI watcher mode desired for the NEXT (re)start: run
    /// `pnpm watch` in the worker's ui/ and set `III_<WORKER>_UI_WATCH=1`.
    /// Meaningful only for workers whose spec has a `ui_dir`; seeded from
    /// `Config::ui_watch` and flipped at runtime by the TUI's `w` key.
    pub ui_watch: bool,
    child: Option<tokio::process::Child>,
    /// The `pnpm watch` companion process (esbuild --watch → ui/dist),
    /// started and stopped with the worker.
    ui_child: Option<tokio::process::Child>,
}

impl WorkerRuntime {
    fn new(ui_watch: bool) -> Self {
        let (log_tx, _) = broadcast::channel(256);
        Self {
            proc_state: ProcState::Stopped,
            exit_code: None,
            started_at: None,
            logs: RingBuffer::new(LOG_RING_CAPACITY),
            log_tx,
            ui_watch,
            child: None,
            ui_child: None,
        }
    }

    /// PID of the spawned child — the `cargo run` supervisor, not the worker
    /// binary it forks. The engine doesn't expose the worker's own pid via
    /// `engine::workers::list`, so this is the closest stable handle.
    // ponytail: supervisor pid is fine for the dashboard; run the built binary
    // directly (instead of `cargo run`) if the true worker pid is ever needed.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(|c| c.id())
    }

    pub fn subscribe_logs(&self) -> broadcast::Receiver<String> {
        self.log_tx.subscribe()
    }

    pub(crate) fn set_child(&mut self, child: tokio::process::Child) {
        self.child = Some(child);
    }

    pub(crate) fn take_child(&mut self) -> Option<tokio::process::Child> {
        self.child.take()
    }

    pub(crate) fn child_mut(&mut self) -> Option<&mut tokio::process::Child> {
        self.child.as_mut()
    }

    pub(crate) fn clear_child(&mut self) {
        self.child = None;
    }

    pub(crate) fn set_ui_child(&mut self, child: tokio::process::Child) {
        self.ui_child = Some(child);
    }

    pub(crate) fn take_ui_child(&mut self) -> Option<tokio::process::Child> {
        self.ui_child.take()
    }
}

pub type SharedRuntimes = Arc<RwLock<HashMap<String, WorkerRuntime>>>;

/// `ui_watch` seeds each worker's initial watcher-mode flag (true only for
/// workers that both ship a ui/ project and have watch enabled globally).
pub fn new_runtimes(workers: &[String], ui_watch: impl Fn(&str) -> bool) -> SharedRuntimes {
    let mut map = HashMap::new();
    for worker in workers {
        map.insert(worker.clone(), WorkerRuntime::new(ui_watch(worker)));
    }
    Arc::new(RwLock::new(map))
}
