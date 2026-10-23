//! One executor thread per session owns every accessibility provider the
//! session uses (on Windows, COM is initialised in MTA on that thread). The
//! async side holds a per-session operation lock for the whole send + wait,
//! so at most one command is ever in flight: a second caller gets `busy`
//! instead of a queue, and a timed-out caller can never see its command
//! dispatched later — the executor is discarded and the next call starts a
//! fresh thread and provider. The stuck thread exits once its native call
//! returns and finds its channel closed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::{oneshot, Mutex};

use super::engine::Engine;
use super::{
    Code, ProviderFactory, Query, SemanticError, SnapshotOut, Verb, ACT_DEADLINE, WALK_DEADLINE,
};

/// Slack the async side gives the executor past the command's own deadline.
const GRACE: Duration = Duration::from_secs(1);

enum Cmd {
    Snapshot {
        query: Query,
        deadline: Instant,
        install_if_empty: bool,
        reply: oneshot::Sender<Result<SnapshotOut, SemanticError>>,
    },
    Act {
        r: String,
        verb: Verb,
        text: Option<String>,
        reply: oneshot::Sender<Result<String, SemanticError>>,
    },
}

struct Exec {
    tx: SyncSender<Cmd>,
    epoch: u64,
}

#[derive(Default)]
struct Slot {
    exec: Option<Exec>,
    next_epoch: u64,
    closed: bool,
}

pub struct SemanticSession {
    factory: ProviderFactory,
    invalidations: Arc<AtomicU64>,
    op: Mutex<()>,
    slot: StdMutex<Slot>,
    walk_deadline: Duration,
    act_deadline: Duration,
}

fn lock(slot: &StdMutex<Slot>) -> MutexGuard<'_, Slot> {
    slot.lock().unwrap_or_else(|p| p.into_inner())
}

fn busy() -> SemanticError {
    SemanticError::new(
        Code::Busy,
        "another accessibility operation is running on this session",
    )
}

/// Discards executor `epoch` when dropped while armed. It lives inside the
/// future the callers await while holding `op`, so it poisons before `op` is
/// released and the next caller can never reach the abandoned executor.
struct PoisonOnDrop<'a> {
    session: &'a SemanticSession,
    epoch: u64,
    armed: bool,
}

impl Drop for PoisonOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.session.poison(self.epoch);
        }
    }
}

impl SemanticSession {
    /// A layer with its own invalidation counter: tests, and any desktop no
    /// other session drives.
    pub fn new(factory: ProviderFactory) -> Self {
        Self::on_desktop(factory, Arc::new(AtomicU64::new(0)))
    }

    /// A layer whose refs die with any physical input counted on
    /// `invalidations`, whichever session on that desktop sent it.
    pub fn on_desktop(factory: ProviderFactory, invalidations: Arc<AtomicU64>) -> Self {
        Self::build(factory, invalidations, WALK_DEADLINE, ACT_DEADLINE)
    }

    #[cfg(test)]
    pub(crate) fn with_deadlines(factory: ProviderFactory, walk: Duration, act: Duration) -> Self {
        Self::build(factory, Arc::new(AtomicU64::new(0)), walk, act)
    }

    fn build(
        factory: ProviderFactory,
        invalidations: Arc<AtomicU64>,
        walk_deadline: Duration,
        act_deadline: Duration,
    ) -> Self {
        Self {
            factory,
            invalidations,
            op: Mutex::new(()),
            slot: StdMutex::new(Slot::default()),
            walk_deadline,
            act_deadline,
        }
    }

    /// Physical input reached this desktop: every ref taken before it, by
    /// any session on the desktop, is stale.
    pub fn invalidate(&self) {
        self.invalidations.fetch_add(1, Ordering::SeqCst);
    }

    /// The desktop's invalidation count: a run that sees it move did not
    /// move it itself (semantic acts never do).
    pub fn invalidations(&self) -> u64 {
        self.invalidations.load(Ordering::SeqCst)
    }

    /// Terminal: later calls fail with `unsupported_session` and never start
    /// a new executor. Never blocks.
    pub fn close(&self) {
        let mut slot = lock(&self.slot);
        slot.closed = true;
        slot.exec = None;
    }

    pub async fn snapshot(
        &self,
        query: Query,
        install_if_empty: bool,
    ) -> Result<SnapshotOut, SemanticError> {
        let _op = self.op.try_lock().map_err(|_| busy())?;
        let deadline = Instant::now() + self.walk_deadline;
        let (reply, rx) = oneshot::channel();
        let epoch = self.send(Cmd::Snapshot {
            query,
            deadline,
            install_if_empty,
            reply,
        })?;
        self.wait(rx, deadline, epoch, Code::BackendError).await
    }

    pub async fn act(
        &self,
        r: String,
        verb: Verb,
        text: Option<String>,
    ) -> Result<String, SemanticError> {
        let _op = self.op.try_lock().map_err(|_| busy())?;
        let deadline = Instant::now() + self.act_deadline;
        let (reply, rx) = oneshot::channel();
        let epoch = self.send(Cmd::Act {
            r,
            verb,
            text,
            reply,
        })?;
        self.wait(rx, deadline, epoch, Code::EffectUnknown).await
    }

    fn send(&self, mut cmd: Cmd) -> Result<u64, SemanticError> {
        let mut slot = lock(&self.slot);
        if slot.closed {
            return Err(SemanticError::new(
                Code::UnsupportedSession,
                "the session was stopped",
            ));
        }
        for _ in 0..2 {
            let exec = match slot.exec.take() {
                Some(exec) => exec,
                None => self.spawn(&mut slot)?,
            };
            match exec.tx.try_send(cmd) {
                Ok(()) => {
                    let epoch = exec.epoch;
                    slot.exec = Some(exec);
                    return Ok(epoch);
                }
                // The thread died (a panic): start a fresh one and retry once.
                Err(TrySendError::Disconnected(back)) => cmd = back,
                // Unreachable while `op` is held (a caller that gives up
                // poisons its executor first); refuse rather than queue.
                Err(TrySendError::Full(_)) => {
                    slot.exec = Some(exec);
                    return Err(busy());
                }
            }
        }
        Err(SemanticError::new(
            Code::BackendError,
            "the accessibility executor could not be started",
        ))
    }

    fn spawn(&self, slot: &mut Slot) -> Result<Exec, SemanticError> {
        let (tx, rx) = sync_channel::<Cmd>(1);
        let mut engine = Engine::new(self.factory.clone(), self.invalidations.clone());
        std::thread::Builder::new()
            .name("computer-a11y".to_string())
            .spawn(move || {
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        Cmd::Snapshot {
                            query,
                            deadline,
                            install_if_empty,
                            reply,
                        } => {
                            if !reply.is_closed() {
                                let _ =
                                    reply.send(engine.snapshot(&query, deadline, install_if_empty));
                            }
                        }
                        Cmd::Act {
                            r,
                            verb,
                            text,
                            reply,
                        } => {
                            if !reply.is_closed() {
                                let _ = reply.send(engine.act(&r, verb, text.as_deref()));
                            }
                        }
                    }
                }
            })
            .map_err(|e| {
                SemanticError::new(
                    Code::BackendError,
                    format!("could not start the accessibility executor: {e}"),
                )
            })?;
        slot.next_epoch += 1;
        Ok(Exec {
            tx,
            epoch: slot.next_epoch,
        })
    }

    async fn wait<T>(
        &self,
        rx: oneshot::Receiver<Result<T, SemanticError>>,
        deadline: Instant,
        epoch: u64,
        lost: Code,
    ) -> Result<T, SemanticError> {
        let budget = deadline.saturating_duration_since(Instant::now()) + GRACE;
        // Poisons on every exit but a delivered reply: a timeout, a dead
        // thread, or this future being dropped (the caller gave up).
        let mut give_up = PoisonOnDrop {
            session: self,
            epoch,
            armed: true,
        };
        match tokio::time::timeout(budget, rx).await {
            Ok(Ok(result)) => {
                give_up.armed = false;
                result
            }
            Ok(Err(_)) => Err(SemanticError::new(
                lost,
                "the accessibility executor stopped mid-operation",
            )),
            Err(_) => Err(SemanticError::new(
                lost,
                "the accessibility call did not return in time",
            )),
        }
    }

    /// Discard executor `epoch` if it is still the current one.
    fn poison(&self, epoch: u64) {
        let mut slot = lock(&self.slot);
        if slot.exec.as_ref().is_some_and(|e| e.epoch == epoch) {
            slot.exec = None;
        }
    }

    #[cfg(test)]
    fn current_epoch(&self) -> Option<u64> {
        lock(&self.slot).exec.as_ref().map(|e| e.epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::fake::FakeTree;
    use crate::semantic::Scope;
    use xa11y_core::Role;

    fn tree() -> Arc<FakeTree> {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        t.add(w, Role::Button, "Salvar");
        Arc::new(t)
    }

    fn observe() -> Query {
        Query {
            scope: Scope::default(),
            selector: None,
            max_nodes: 256,
            max_depth: 6,
        }
    }

    fn salvar_ref(text: &str) -> String {
        let line = text.lines().find(|l| l.contains("Salvar")).unwrap();
        line.trim_start()
            .strip_prefix("[ref=")
            .unwrap()
            .split(']')
            .next()
            .unwrap()
            .to_string()
    }

    fn fast(t: &Arc<FakeTree>) -> SemanticSession {
        SemanticSession::with_deadlines(
            t.factory(),
            Duration::from_millis(300),
            Duration::from_millis(200),
        )
    }

    #[tokio::test]
    async fn sequential_commands_are_never_busy() {
        let t = tree();
        let s = SemanticSession::new(t.factory());
        for _ in 0..50 {
            s.snapshot(observe(), true).await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_second_operation_is_busy_and_never_queued() {
        let t = tree();
        let s = Arc::new(SemanticSession::new(t.factory()));
        let first = s.snapshot(observe(), true).await.unwrap();
        let r = salvar_ref(&first.text);
        let before = t.children_calls();
        t.block_reads.store(true, Ordering::SeqCst);
        let bg = s.clone();
        let walking = tokio::spawn(async move { bg.snapshot(observe(), true).await });
        while t.children_calls() <= before {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let err = s.act(r, Verb::Invoke, None).await.unwrap_err();
        assert_eq!(err.code, Code::Busy);
        t.block_reads.store(false, Ordering::SeqCst);
        walking.await.unwrap().unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(t.actions().is_empty(), "the refused act never ran later");
    }

    #[tokio::test]
    async fn a_hung_action_is_effect_unknown_and_the_next_call_gets_a_fresh_executor() {
        let t = tree();
        let s = fast(&t);
        let r = salvar_ref(&s.snapshot(observe(), true).await.unwrap().text);
        t.block_actions.store(true, Ordering::SeqCst);
        let err = s.act(r.clone(), Verb::Invoke, None).await.unwrap_err();
        assert_eq!(err.code, Code::EffectUnknown);
        t.block_actions.store(false, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(50)).await;
        // The hung call itself, and nothing dispatched after the timeout.
        assert_eq!(t.actions().len(), 1, "{:?}", t.actions());
        // New executor: the old snapshot (and its refs) went with the old one.
        let err = s.act(r, Verb::Invoke, None).await.unwrap_err();
        assert_eq!(err.code, Code::StaleRef);
        s.snapshot(observe(), true).await.unwrap();
    }

    #[tokio::test]
    async fn a_cancelled_caller_discards_the_executor_like_a_timeout() {
        let t = tree();
        let s = fast(&t);
        let r = salvar_ref(&s.snapshot(observe(), true).await.unwrap().text);
        t.block_actions.store(true, Ordering::SeqCst);
        // The caller gives up (outer timeout, select!, task abort) only once
        // the press is in the provider, so the run never depends on timing.
        tokio::select! {
            res = s.act(r, Verb::Invoke, None) => panic!("the act must still be hung: {res:?}"),
            () = async {
                while t.actions().is_empty() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            } => {}
        }
        assert_eq!(s.current_epoch(), None, "the wedged executor was discarded");
        t.block_actions.store(false, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Only the hung press itself; nothing was queued behind it.
        assert_eq!(t.actions().len(), 1, "{:?}", t.actions());
        // The next call gets a fresh executor at once.
        tokio::time::timeout(Duration::from_secs(1), s.snapshot(observe(), true))
            .await
            .expect("a fresh executor answers promptly")
            .unwrap();
    }

    #[tokio::test]
    async fn a_hung_read_is_backend_error() {
        let t = tree();
        let s = fast(&t);
        t.block_reads.store(true, Ordering::SeqCst);
        let err = s.snapshot(observe(), true).await.unwrap_err();
        assert_eq!(err.code, Code::BackendError);
        t.block_reads.store(false, Ordering::SeqCst);
        s.snapshot(observe(), true).await.unwrap();
    }

    #[tokio::test]
    async fn a_provider_panic_is_effect_unknown_and_recoverable() {
        let t = tree();
        let s = SemanticSession::new(t.factory());
        let r = salvar_ref(&s.snapshot(observe(), true).await.unwrap().text);
        t.panic_on_action.store(true, Ordering::SeqCst);
        assert_eq!(
            s.act(r.clone(), Verb::Invoke, None).await.unwrap_err().code,
            Code::EffectUnknown
        );
        t.panic_on_action.store(false, Ordering::SeqCst);
        assert_eq!(
            s.act(r, Verb::Invoke, None).await.unwrap_err().code,
            Code::StaleRef
        );
        s.snapshot(observe(), true).await.unwrap();
    }

    #[tokio::test]
    async fn a_command_whose_caller_gave_up_is_never_dispatched() {
        let t = tree();
        let s = SemanticSession::new(t.factory());
        let r = salvar_ref(&s.snapshot(observe(), true).await.unwrap().text);
        let (reply, rx) = oneshot::channel();
        drop(rx);
        s.send(Cmd::Act {
            r,
            verb: Verb::Invoke,
            text: None,
            reply,
        })
        .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(t.actions().is_empty(), "{:?}", t.actions());
    }

    #[tokio::test]
    async fn poisoning_an_old_epoch_keeps_the_current_executor() {
        let t = tree();
        let s = SemanticSession::new(t.factory());
        s.snapshot(observe(), true).await.unwrap();
        let live = s.current_epoch().unwrap();
        s.poison(live + 100);
        assert_eq!(s.current_epoch(), Some(live));
        s.poison(live);
        assert_eq!(s.current_epoch(), None);
    }

    #[tokio::test]
    async fn close_is_terminal() {
        let t = tree();
        let s = SemanticSession::new(t.factory());
        s.snapshot(observe(), true).await.unwrap();
        s.close();
        let err = s.snapshot(observe(), true).await.unwrap_err();
        assert_eq!(err.code, Code::UnsupportedSession);
        assert!(err.message.contains("session was stopped"));
        assert_eq!(s.current_epoch(), None);
    }

    #[tokio::test]
    async fn invalidate_kills_refs() {
        let t = tree();
        let s = SemanticSession::new(t.factory());
        let r = salvar_ref(&s.snapshot(observe(), true).await.unwrap().text);
        s.invalidate();
        assert_eq!(
            s.act(r, Verb::Invoke, None).await.unwrap_err().code,
            Code::StaleRef
        );
    }
}
