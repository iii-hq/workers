//! Named, durable native sessions. Opening a manager never opens a WhatsApp connection.
use crate::{config::Config, inbox::Inbox, registry::Registry};
use anyhow::{Context, Result, bail, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex as AsyncMutex, broadcast};
use tokio::task::JoinHandle;
use whatsapp_rust::Client;
use whatsapp_rust::pair_code::PairCodeOptions;
use whatsapp_rust::prelude::{Bot, Event as NativeEvent, EventHandler, SqliteStore};

const SEQUENCE_BLOCK: u64 = 1024;
const MAX_EVENT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Stopped,
    Starting,
    Pairing,
    Connected,
    Disconnected,
    Stopping,
    LoggedOut,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SessionSummary {
    pub session_id: String,
    pub status: SessionStatus,
    pub created_at_ms: u64,
}

/// Pairing material is returned only by the dedicated pairing functions.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PairingCode {
    pub code: String,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    pub session_id: String,
    pub sequence: u64,
    pub kind: String,
    pub timestamp_ms: u64,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EventPage {
    pub events: Vec<Event>,
    /// Pass this value as `after_sequence` in the next poll.
    pub next_sequence: u64,
    /// Earlier events have fallen outside the bounded queue or a process restarted.
    pub truncated: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    session_id: String,
    created_at_ms: u64,
    /// Reserved high watermark: unused numbers are skipped after process restart.
    next_sequence: u64,
}

struct State {
    status: SessionStatus,
    generation: u64,
    deleted: bool,
    qr: Option<PairingCode>,
    pair_code: Option<PairingCode>,
    pair_phone: Option<String>,
    next_sequence: u64,
    reserved_until: u64,
    events: VecDeque<Event>,
}

struct Running {
    client: Arc<Client>,
    store: Weak<SqliteStore>,
    task: JoinHandle<()>,
}

impl Drop for Running {
    fn drop(&mut self) {
        // Also covers process-owner/RPC shutdown cancellation after taking the handle.
        self.task.abort();
    }
}

#[derive(Default)]
struct Lifecycle {
    running: Option<Running>,
    /// Never reopen/remove SQLite files while an in-flight operation owns the old client.
    retired: Option<(Weak<Client>, Weak<SqliteStore>)>,
}

struct Session {
    id: String,
    directory: PathBuf,
    created_at_ms: u64,
    max_events: usize,
    inbox: Arc<Inbox>,
    state: Mutex<State>,
    lifecycle: AsyncMutex<Lifecycle>,
    broadcast: broadcast::Sender<Event>,
}

impl Session {
    fn from_metadata(
        directory: PathBuf,
        metadata: Metadata,
        max_events: usize,
        max_inbox_messages: usize,
        broadcast: broadcast::Sender<Event>,
    ) -> Result<Self> {
        let inbox = Inbox::open(&directory, max_inbox_messages)?;
        Ok(Self {
            id: metadata.session_id,
            directory,
            created_at_ms: metadata.created_at_ms,
            max_events,
            inbox,
            state: Mutex::new(State {
                status: SessionStatus::Stopped,
                generation: 0,
                deleted: false,
                qr: None,
                pair_code: None,
                pair_phone: None,
                next_sequence: metadata.next_sequence,
                reserved_until: metadata.next_sequence,
                events: VecDeque::new(),
            }),
            lifecycle: AsyncMutex::new(Lifecycle::default()),
            broadcast,
        })
    }

    fn summary(&self) -> SessionSummary {
        SessionSummary {
            session_id: self.id.clone(),
            status: self.state.lock().unwrap().status.clone(),
            created_at_ms: self.created_at_ms,
        }
    }

    fn record(&self, state: &mut State, kind: &str, data: Value) -> Result<Event> {
        ensure!(!state.deleted, "session has been deleted");
        let event = Event {
            session_id: self.id.clone(),
            sequence: state.next_sequence,
            kind: kind.to_owned(),
            timestamp_ms: now_ms(),
            data,
        };
        ensure!(
            serde_json::to_vec(&event)?.len() <= MAX_EVENT_BYTES,
            "event exceeds maximum payload size"
        );
        if state.next_sequence == state.reserved_until {
            let until = state
                .next_sequence
                .checked_add(SEQUENCE_BLOCK)
                .context("session sequence exhausted")?;
            persist_metadata(
                &self.directory,
                &Metadata {
                    session_id: self.id.clone(),
                    created_at_ms: self.created_at_ms,
                    next_sequence: until,
                },
            )?;
            state.reserved_until = until;
        }
        state.next_sequence += 1;
        if state.events.len() == self.max_events {
            state.events.pop_front();
        }
        state.events.push_back(event.clone());
        let _ = self.broadcast.send(event.clone());
        Ok(event)
    }

    fn transition(&self, status: SessionStatus, kind: &str) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.status = status.clone();
        state.qr = None;
        state.pair_code = None;
        state.pair_phone = None;
        self.record(&mut state, kind, json!({"status": status}))?;
        Ok(())
    }

    async fn wait_retired(lifecycle: &mut Lifecycle, timeout: Duration) -> Result<()> {
        if let Some((client, store)) = &lifecycle.retired {
            tokio::time::timeout(timeout, async {
                while client.strong_count() != 0 || store.strong_count() != 0 {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("session still has in-flight native operations; retry after they finish")?;
        }
        lifecycle.retired = None;
        Ok(())
    }

    async fn stop_locked(&self, lifecycle: &mut Lifecycle, timeout: Duration) -> Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            state.generation += 1;
            self.inbox.invalidate();
            state.qr = None;
            state.pair_code = None;
            state.pair_phone = None;
        }
        if let Some(mut running) = lifecycle.running.take() {
            {
                let mut state = self.state.lock().unwrap();
                state.status = SessionStatus::Stopping;
            }
            lifecycle.retired = Some((Arc::downgrade(&running.client), running.store.clone()));
            let stopped = tokio::time::timeout(timeout, async {
                running.client.disconnect().await;
                let _ = (&mut running.task).await;
            })
            .await;
            if stopped.is_err() {
                running.task.abort();
                let _ = (&mut running.task).await;
                drop(running);
                self.transition(SessionStatus::Failed, "session.stop_timeout")?;
                bail!("native session shutdown timed out; credentials were preserved");
            }
            drop(running);
        }
        if self.state.lock().unwrap().status != SessionStatus::Stopped {
            self.transition(SessionStatus::Stopped, "session.stopped")?;
        }
        Self::wait_retired(lifecycle, timeout).await
    }
}

pub struct SessionManager {
    config: RwLock<Config>,
    sessions: RwLock<BTreeMap<String, Arc<Session>>>,
    broadcast: broadcast::Sender<Event>,
    shutting_down: AtomicBool,
}

impl SessionManager {
    pub fn open(config: Config) -> Result<Arc<Self>> {
        config.validate()?;
        private_directory(&config.data_dir)?;
        let (broadcast, _) = broadcast::channel(config.max_events);
        let mut sessions = BTreeMap::new();
        for entry in fs::read_dir(&config.data_dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            ensure!(!file_type.is_symlink(), "symlink in session data directory");
            if !file_type.is_dir() {
                continue;
            }
            let id = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 session directory"))?;
            validate_id(&id)?;
            let path = entry.path();
            private_directory(&path)?;
            let metadata_path = path.join("metadata.json");
            regular_file(&metadata_path)?;
            let bytes = fs::read(&metadata_path).context("read session metadata")?;
            ensure!(bytes.len() <= 4096, "session metadata exceeds limit");
            let metadata: Metadata =
                serde_json::from_slice(&bytes).context("invalid session metadata")?;
            ensure!(
                metadata.session_id == id && metadata.next_sequence > 0,
                "session metadata does not match its directory"
            );
            sessions.insert(
                id,
                Arc::new(Session::from_metadata(
                    path,
                    metadata,
                    config.max_events,
                    config.max_inbox_messages,
                    broadcast.clone(),
                )?),
            );
        }
        Ok(Arc::new(Self {
            config: RwLock::new(config),
            sessions: RwLock::new(sessions),
            broadcast,
            shutting_down: AtomicBool::new(false),
        }))
    }

    pub fn config(&self) -> Config {
        self.config.read().unwrap().clone()
    }

    /// Apply tuning without moving open SQLite stores or replacing event channels.
    /// The returned fields retain their boot values until the worker restarts.
    pub fn apply_config(&self, mut config: Config) -> Result<Vec<String>> {
        config.validate()?;
        // Match create's sessions -> config lock order and retain the guard so
        // concurrent creation/deletion cannot miss a live archive-limit change.
        let sessions = self.sessions.read().unwrap();
        let mut active = self.config.write().unwrap();
        let mut restart_required = Vec::new();
        if config.data_dir != active.data_dir {
            restart_required.push("data_dir".to_owned());
            config.data_dir = active.data_dir.clone();
        }
        if config.max_events != active.max_events {
            restart_required.push("max_events".to_owned());
            config.max_events = active.max_events;
        }
        if config.max_inbox_messages != active.max_inbox_messages {
            for session in sessions.values() {
                session.inbox.set_limit(config.max_inbox_messages)?;
            }
        }
        *active = config;
        Ok(restart_required)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.broadcast.subscribe()
    }

    /// Local display archive access never starts or connects a native session.
    pub fn inbox(&self, id: &str) -> Result<Arc<Inbox>> {
        let session = self.session(id)?;
        ensure!(
            !session.state.lock().unwrap().deleted,
            "session has been deleted"
        );
        Ok(session.inbox.clone())
    }

    fn active(&self) -> Result<()> {
        ensure!(
            !self.shutting_down.load(Ordering::Acquire),
            "worker is shutting down"
        );
        Ok(())
    }

    fn session(&self, id: &str) -> Result<Arc<Session>> {
        validate_id(id)?;
        self.sessions
            .read()
            .unwrap()
            .get(id)
            .cloned()
            .with_context(|| format!("unknown session: {id}"))
    }

    pub async fn create(&self, id: &str) -> Result<SessionSummary> {
        validate_id(id)?;
        let mut sessions = self.sessions.write().unwrap();
        let config = self.config();
        self.active()?;
        ensure!(!sessions.contains_key(id), "session already exists: {id}");
        ensure!(
            sessions.len() < config.max_sessions,
            "maximum session count reached"
        );
        let directory = config.data_dir.join(id);
        fs::create_dir(&directory).context("create session directory")?;
        let metadata = Metadata {
            session_id: id.to_owned(),
            created_at_ms: now_ms(),
            next_sequence: 1,
        };
        if let Err(error) =
            private_directory(&directory).and_then(|()| persist_metadata(&directory, &metadata))
        {
            let _ = fs::remove_dir_all(&directory);
            return Err(error);
        }
        let session = match Session::from_metadata(
            directory.clone(),
            metadata,
            config.max_events,
            config.max_inbox_messages,
            self.broadcast.clone(),
        ) {
            Ok(session) => Arc::new(session),
            Err(error) => {
                let _ = fs::remove_dir_all(&directory);
                return Err(error.context("initialize session inbox"));
            }
        };
        let summary = session.summary();
        sessions.insert(id.to_owned(), session);
        Ok(summary)
    }

    pub async fn get(&self, id: &str) -> Result<SessionSummary> {
        Ok(self.session(id)?.summary())
    }

    pub async fn list(&self) -> Vec<SessionSummary> {
        self.sessions
            .read()
            .unwrap()
            .values()
            .map(|session| session.summary())
            .collect()
    }

    /// Mutating lifecycle work owns its task so cancellation of an RPC cannot leak a bot.
    pub async fn start(self: &Arc<Self>, id: &str) -> Result<SessionSummary> {
        self.active()?;
        let session = self.session(id)?;
        let manager = self.clone();
        tokio::spawn(async move { manager.start_session(session).await }).await?
    }

    async fn start_session(&self, session: Arc<Session>) -> Result<SessionSummary> {
        let mut lifecycle = session.lifecycle.lock().await;
        self.active()?;
        ensure!(
            !session.state.lock().unwrap().deleted,
            "session has been deleted"
        );
        if lifecycle
            .running
            .as_ref()
            .is_some_and(|running| !running.task.is_finished())
        {
            return Ok(session.summary());
        }
        let config = self.config();
        let timeout = Duration::from_secs(config.shutdown_timeout_secs);
        session.stop_locked(&mut lifecycle, timeout).await?;
        let generation = {
            let mut state = session.state.lock().unwrap();
            state.generation += 1;
            state.generation
        };
        session.transition(SessionStatus::Starting, "session.starting")?;
        let built =
            tokio::time::timeout(Duration::from_secs(config.operation_timeout_secs), async {
                private_directory(&session.directory)?;
                let path = session.directory.join("credentials.db");
                private_file(&path)?;
                for suffix in ["credentials.db-wal", "credentials.db-shm"] {
                    let companion = session.directory.join(suffix);
                    if companion.exists() {
                        regular_file(&companion)?;
                    }
                }
                let store = Arc::new(
                    SqliteStore::new(
                        path.to_str()
                            .context("session data directory must be UTF-8")?,
                    )
                    .await?,
                );
                let bot = Bot::builder()
                    .with_backend_arc(store.clone())
                    .with_inbound_durability_hook(NativeArchive {
                        session: Arc::downgrade(&session),
                        generation,
                    })
                    .with_event_handler(NativeEvents {
                        session: Arc::downgrade(&session),
                        generation,
                    })
                    .build()
                    .await?;
                if let Some(account) = bot.client().pn() {
                    session
                        .inbox
                        .set_account(&account.to_non_ad().to_string())?;
                } else {
                    // Credentials were removed or this is a new pairing. An
                    // archive with unknown ownership cannot follow a new account.
                    session.inbox.clear()?;
                }
                for name in ["credentials.db", "credentials.db-wal", "credentials.db-shm"] {
                    let file = session.directory.join(name);
                    if file.exists() {
                        private_file(&file)?;
                    }
                }
                Ok::<_, anyhow::Error>((bot, Arc::downgrade(&store)))
            })
            .await;
        let (bot, store) = match built {
            Ok(Ok(value)) => value,
            result => {
                session.transition(SessionStatus::Failed, "session.start_failed")?;
                return match result {
                    Ok(Err(error)) => Err(error.context("initialize native session")),
                    Err(error) => Err(error.into()),
                    Ok(Ok(_)) => unreachable!(),
                };
            }
        };
        let client = bot.client();
        self.active()?;
        let weak = Arc::downgrade(&session);
        let task = tokio::spawn(async move {
            bot.run().await;
            if let Some(session) = weak.upgrade() {
                let mut state = session.state.lock().unwrap();
                if state.generation == generation && !state.deleted {
                    if state.status != SessionStatus::LoggedOut {
                        state.status = SessionStatus::Disconnected;
                    }
                    state.qr = None;
                    state.pair_code = None;
                    state.pair_phone = None;
                    let _ = session.record(&mut state, "session.ended", json!({}));
                }
            }
        });
        lifecycle.running = Some(Running {
            client,
            store,
            task,
        });
        Ok(session.summary())
    }

    pub async fn client(&self, id: &str) -> Result<Arc<Client>> {
        self.active()?;
        let session = self.session(id)?;
        let lifecycle = session.lifecycle.lock().await;
        let state = session.state.lock().unwrap();
        ensure!(
            !state.deleted && state.status == SessionStatus::Connected,
            "session is not connected: {id}"
        );
        let running = lifecycle
            .running
            .as_ref()
            .context("session is not running")?;
        ensure!(
            !running.task.is_finished() && running.client.is_connected(),
            "session is not connected: {id}"
        );
        Ok(running.client.clone())
    }

    pub async fn stop(self: &Arc<Self>, id: &str) -> Result<SessionSummary> {
        let session = self.session(id)?;
        let timeout = Duration::from_secs(self.config().shutdown_timeout_secs);
        tokio::spawn(async move {
            let mut lifecycle = session.lifecycle.lock().await;
            ensure!(
                !session.state.lock().unwrap().deleted,
                "session has been deleted"
            );
            session.stop_locked(&mut lifecycle, timeout).await?;
            Ok(session.summary())
        })
        .await?
    }

    pub async fn qr(&self, id: &str) -> Result<Option<PairingCode>> {
        let session = self.session(id)?;
        let mut state = session.state.lock().unwrap();
        if state
            .qr
            .as_ref()
            .is_some_and(|qr| qr.expires_at_ms <= now_ms())
        {
            state.qr = None;
        }
        Ok(state.qr.clone())
    }

    pub async fn pair_code(&self, id: &str, phone_number: &str) -> Result<PairingCode> {
        self.active()?;
        ensure!(
            (7..=15).contains(&phone_number.len())
                && phone_number.bytes().all(|byte| byte.is_ascii_digit()),
            "phone_number must contain 7 to 15 digits including country code"
        );
        let session = self.session(id)?;
        let lifecycle = session.lifecycle.lock().await;
        {
            let mut state = session.state.lock().unwrap();
            ensure!(
                !state.deleted
                    && matches!(
                        state.status,
                        SessionStatus::Starting | SessionStatus::Pairing
                    ),
                "session must be started and awaiting pairing"
            );
            if let Some(code) = &state.pair_code {
                if code.expires_at_ms > now_ms() {
                    ensure!(
                        state.pair_phone.as_deref() == Some(phone_number),
                        "a pairing code is already active for another request"
                    );
                    return Ok(code.clone());
                }
                state.pair_code = None;
                state.pair_phone = None;
            }
            state.pair_phone = Some(phone_number.to_owned());
        }
        let running = lifecycle
            .running
            .as_ref()
            .context("session is not running")?;
        let code = tokio::time::timeout(
            Duration::from_secs(self.config().operation_timeout_secs),
            running.client.pair_with_code(PairCodeOptions {
                phone_number: phone_number.to_owned(),
                ..Default::default()
            }),
        )
        .await
        .context("pair-code request timed out")??;
        // The native event handler stores the authoritative expiry synchronously.
        let state = session.state.lock().unwrap();
        state
            .pair_code
            .clone()
            .filter(|pair| pair.code == code && pair.expires_at_ms > now_ms())
            .context("pairing code was invalidated before the request completed")
    }

    pub async fn logout(self: &Arc<Self>, id: &str) -> Result<SessionSummary> {
        self.active()?;
        let session = self.session(id)?;
        let config = self.config();
        let timeout = Duration::from_secs(config.shutdown_timeout_secs);
        let operation_timeout = Duration::from_secs(config.operation_timeout_secs);
        tokio::spawn(async move {
            let mut lifecycle = session.lifecycle.lock().await;
            ensure!(
                session.state.lock().unwrap().status == SessionStatus::Connected,
                "logout requires a connected session so the native unlink request can be sent"
            );
            let running = lifecycle
                .running
                .as_ref()
                .context("session is not running")?;
            ensure!(
                running.client.is_connected(),
                "logout requires a connected session"
            );
            // Native logout is best effort: its API cannot confirm remote deregistration.
            let unlinked = tokio::time::timeout(operation_timeout, running.client.logout()).await;
            session.stop_locked(&mut lifecycle, timeout).await?;
            unlinked.context("native logout timed out; local credentials were retained")?;
            clear_credentials(&session.directory)?;
            session.inbox.clear()?;
            session.transition(SessionStatus::LoggedOut, "session.logged_out")?;
            Ok(session.summary())
        })
        .await?
    }

    pub async fn delete(self: &Arc<Self>, id: &str) -> Result<SessionSummary> {
        let session = self.session(id)?;
        let manager = self.clone();
        tokio::spawn(async move {
            let mut lifecycle = session.lifecycle.lock().await;
            ensure!(
                !session.state.lock().unwrap().deleted,
                "session has been deleted"
            );
            session
                .stop_locked(
                    &mut lifecycle,
                    Duration::from_secs(manager.config().shutdown_timeout_secs),
                )
                .await?;
            let mut sessions = manager.sessions.write().unwrap();
            private_directory(&session.directory)?;
            session.inbox.close()?;
            fs::remove_dir_all(&session.directory).context("remove session data")?;
            session.state.lock().unwrap().deleted = true;
            sessions.remove(&session.id);
            Ok(session.summary())
        })
        .await?
    }

    pub async fn publish(&self, id: &str, kind: &str, data: Value) -> Result<Event> {
        ensure!(
            !kind.is_empty()
                && kind.len() <= 100
                && kind
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)),
            "invalid event kind"
        );
        let session = self.session(id)?;
        let mut state = session.state.lock().unwrap();
        session.record(&mut state, kind, data)
    }

    pub async fn events(&self, id: &str, after_sequence: u64, limit: usize) -> Result<EventPage> {
        ensure!(limit > 0, "event limit must be positive");
        let session = self.session(id)?;
        let state = session.state.lock().unwrap();
        let events: Vec<_> = state
            .events
            .iter()
            .filter(|event| event.sequence > after_sequence)
            .take(limit.min(session.max_events))
            .cloned()
            .collect();
        let truncated = state.events.front().map_or(
            state.next_sequence > after_sequence.saturating_add(1),
            |first| first.sequence > after_sequence.saturating_add(1),
        );
        let next_sequence = events.last().map_or(
            after_sequence.max(state.next_sequence.saturating_sub(1)),
            |event| event.sequence,
        );
        Ok(EventPage {
            events,
            next_sequence,
            truncated,
        })
    }

    pub async fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::Release);
        let sessions: Vec<_> = self.sessions.read().unwrap().values().cloned().collect();
        let mut tasks = tokio::task::JoinSet::new();
        let timeout = Duration::from_secs(self.config().shutdown_timeout_secs);
        for session in sessions {
            tasks.spawn(async move {
                let stopped = tokio::time::timeout(timeout, async {
                    let mut lifecycle = session.lifecycle.lock().await;
                    if !session.state.lock().unwrap().deleted {
                        session.stop_locked(&mut lifecycle, timeout).await?;
                    }
                    Ok::<_, anyhow::Error>(())
                })
                .await;
                if !matches!(stopped, Ok(Ok(()))) {
                    tracing::warn!("a native WhatsApp session did not finish shutdown cleanly");
                }
            });
        }
        while tasks.join_next().await.is_some() {}
    }
}

struct NativeEvents {
    session: Weak<Session>,
    generation: u64,
}

struct NativeArchive {
    session: Weak<Session>,
    generation: u64,
}

#[async_trait::async_trait]
impl whatsapp_rust::types::durability_hook::InboundDurabilityHook for NativeArchive {
    async fn on_messages(
        &self,
        client: Arc<Client>,
        batch: &[whatsapp_rust::prelude::InboundMessage],
    ) -> Result<()> {
        let session = self
            .session
            .upgrade()
            .context("session archive is unavailable")?;
        let capture = {
            let state = session.state.lock().unwrap();
            ensure!(
                !state.deleted && state.generation == self.generation,
                "session archive generation changed"
            );
            session.inbox.capture()
        };
        capture.on_messages(client, batch).await
    }
}

impl EventHandler for NativeEvents {
    fn handle_event(&self, event: Arc<NativeEvent>) {
        let Some(session) = self.session.upgrade() else {
            return;
        };
        let mut state = session.state.lock().unwrap();
        if state.deleted || state.generation != self.generation {
            return;
        }
        if matches!(
            &*event,
            NativeEvent::HistorySync(_) | NativeEvent::Receipt(_)
        ) || matches!(&*event, NativeEvent::Messages(batch) if !batch.hook_committed)
        {
            session.inbox.capture().ingest_event(event.clone());
        }
        let payload = match &*event {
            NativeEvent::PairingQrCode(qr) => {
                let expiry =
                    now_ms().saturating_add(qr.timeout.as_millis().try_into().unwrap_or(u64::MAX));
                state.status = SessionStatus::Pairing;
                state.qr = Some(PairingCode {
                    code: qr.code.clone(),
                    expires_at_ms: expiry,
                });
                Some(("session.qr", json!({"expires_at_ms": expiry})))
            }
            NativeEvent::PairingCode(pair) => {
                let expiry = now_ms()
                    .saturating_add(pair.timeout.as_millis().try_into().unwrap_or(u64::MAX));
                state.pair_code = Some(PairingCode {
                    code: pair.code.clone(),
                    expires_at_ms: expiry,
                });
                Some(("session.pair_code", json!({"expires_at_ms": expiry})))
            }
            NativeEvent::Connected(_) => {
                state.status = SessionStatus::Connected;
                state.qr = None;
                state.pair_code = None;
                state.pair_phone = None;
                Some(("session.connected", json!({})))
            }
            NativeEvent::Disconnected(disconnected) => {
                state.status = SessionStatus::Disconnected;
                state.qr = None;
                state.pair_code = None;
                state.pair_phone = None;
                Some((
                    "session.disconnected",
                    json!({"reason": disconnected.reason}),
                ))
            }
            NativeEvent::LoggedOut(logout) => {
                if session.inbox.clear().is_err() {
                    tracing::warn!("could not clear logged-out WhatsApp inbox");
                }
                state.status = SessionStatus::LoggedOut;
                state.qr = None;
                state.pair_code = None;
                state.pair_phone = None;
                Some((
                    "session.logged_out",
                    json!({"on_connect": logout.on_connect, "reason": logout.reason}),
                ))
            }
            NativeEvent::PairingCodeRefresh(_) | NativeEvent::PairingCodeError(_) => {
                state.pair_code = None;
                state.pair_phone = None;
                Some(("session.pair_code_invalidated", json!({})))
            }
            NativeEvent::PairingQrCodesExhausted(_) => {
                state.qr = None;
                Some(("session.qr_expired", json!({})))
            }
            NativeEvent::PairSuccess(pair) => {
                if session
                    .inbox
                    .set_account(&pair.id.to_non_ad().to_string())
                    .is_err()
                {
                    tracing::warn!("could not update paired WhatsApp inbox account");
                }
                state.qr = None;
                state.pair_code = None;
                state.pair_phone = None;
                Some(("session.paired", json!({"id": pair.id, "lid": pair.lid})))
            }
            NativeEvent::PairError(_) => {
                state.qr = None;
                state.pair_code = None;
                state.pair_phone = None;
                Some(("session.pairing_failed", json!({})))
            }
            NativeEvent::Messages(batch) => {
                for message in batch.iter() {
                    let info = json!({
                        "id": message.info.id.to_string(),
                        "source": {
                            "chat": message.info.source.chat.to_string(),
                            "sender": message.info.source.sender.to_string(),
                            "is_from_me": message.info.source.is_from_me,
                            "is_group": message.info.source.is_group,
                            "sender_alt": message.info.source.sender_alt.as_ref().map(ToString::to_string),
                            "recipient_alt": message.info.source.recipient_alt.as_ref().map(ToString::to_string),
                        },
                        "timestamp_ms": message.info.timestamp.timestamp_millis(),
                        "push_name": message.info.push_name,
                        "is_offline": message.info.is_offline,
                    });
                    let mut data = json!({"info": info, "message": message.message, "ephemeral_expiration": message.ephemeral_expiration, "comment_target": message.comment_target});
                    // Leave room for the envelope and retain routing information for large media.
                    if serde_json::to_vec(&data)
                        .map_or(true, |bytes| bytes.len() > MAX_EVENT_BYTES - 512)
                    {
                        data = json!({"info": info, "content_omitted": true, "reason": "event_size_limit"});
                    }
                    if session
                        .record(&mut state, "message.received", data)
                        .is_err()
                    {
                        tracing::warn!("could not retain native WhatsApp message event");
                    }
                }
                None
            }
            NativeEvent::Receipt(value) => Some(("message.receipt", json!(value))),
            NativeEvent::IncomingCall(call) => {
                use whatsapp_rust::types::call::CallAction;
                let (is_video, is_group, kind) = match &call.action {
                    CallAction::Offer {
                        is_video,
                        group_jid,
                        ..
                    } => (Some(*is_video), Some(group_jid.is_some()), "call.incoming"),
                    CallAction::OfferNotice {
                        is_video, is_group, ..
                    } => (Some(*is_video), Some(*is_group), "call.incoming"),
                    _ => (None, None, "call.updated"),
                };
                Some((
                    kind,
                    json!({
                        "call_id": call.action.call_id(),
                        "peer": call.from.to_string(),
                        "call_creator": call.action.call_creator().to_string(),
                        "action": call.action.wire_tag(),
                        "is_video": is_video,
                        "is_group": is_group,
                        "timestamp_ms": call.timestamp.timestamp_millis(),
                        "offline": call.offline,
                    }),
                ))
            }
            NativeEvent::MissedCall(call) => Some((
                "call.missed",
                json!({
                    "call_id": call.call_id, "peer": call.from.to_string(),
                    "timestamp_ms": call.timestamp.timestamp_millis(), "reason": call.reason,
                }),
            )),
            NativeEvent::CallEndedElsewhere(call) => Some((
                "call.ended_elsewhere",
                json!({
                    "call_id": call.call_id, "peer": call.from.to_string(),
                    "timestamp_ms": call.timestamp.timestamp_millis(), "outcome": call.outcome,
                }),
            )),
            NativeEvent::ServerAck(value) => Some(("message.ack", json!(value))),
            NativeEvent::UndecryptableMessage(value) => {
                Some(("message.undecryptable", json!(value)))
            }
            NativeEvent::Presence(value) => Some(("presence.updated", json!(value))),
            NativeEvent::ChatPresence(value) => Some(("chat.presence", json!(value))),
            NativeEvent::GroupUpdate(value) => Some(("group.updated", json!(value))),
            NativeEvent::ContactUpdated(value) => Some(("contact.updated", json!(value))),
            NativeEvent::ContactNumberChanged(value) => {
                Some(("contact.number_changed", json!(value)))
            }
            NativeEvent::PictureUpdate(value) => Some(("profile.picture_updated", json!(value))),
            NativeEvent::UserAboutUpdate(value) => Some(("profile.about_updated", json!(value))),
            NativeEvent::NewsletterLiveUpdate(value) => Some(("newsletter.updated", json!(value))),
            NativeEvent::LabelEditUpdate(value) => Some(("label.updated", json!(value))),
            NativeEvent::LabelAssociationUpdate(value) => {
                Some(("label.association_updated", json!(value)))
            }
            NativeEvent::HistorySync(value) => Some(("history.sync", json!(value))),
            NativeEvent::OfflineSyncCompleted(value) => Some(("history.completed", json!(value))),
            _ => None,
        };
        if let Some((kind, data)) = payload
            && session.record(&mut state, kind, data).is_err()
        {
            tracing::warn!("could not retain native WhatsApp event");
        }
    }
}

fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte)),
        "session_id must contain 1 to 64 ASCII letters, digits, underscores or hyphens"
    );
    Ok(())
}

/// Import one existing session's private event snapshot offline. Other session
/// directories and archives are neither opened nor trimmed by this operation.
pub fn import_events(config: &Config, id: &str, page: &Value) -> Result<usize> {
    config.validate()?;
    validate_id(id)?;
    ensure!(
        config.data_dir.exists(),
        "session data directory does not exist"
    );
    private_directory(&config.data_dir)?;
    let directory = config.data_dir.join(id);
    ensure!(directory.exists(), "unknown session: {id}");
    private_directory(&directory)?;
    let metadata_path = directory.join("metadata.json");
    regular_file(&metadata_path)?;
    ensure!(
        fs::metadata(&metadata_path)?.len() <= 4096,
        "session metadata exceeds limit"
    );
    let metadata: Metadata = serde_json::from_slice(&fs::read(metadata_path)?)?;
    ensure!(
        metadata.session_id == id && metadata.next_sequence > 0,
        "session metadata does not match its directory"
    );
    let events = page
        .as_array()
        .or_else(|| page.get("events").and_then(Value::as_array))
        .context("expected an event array or event page")?;
    for event in events {
        if let Some(event_session) = event.get("session_id") {
            ensure!(
                event_session.as_str() == Some(id),
                "snapshot contains events for another session"
            );
        }
    }
    Inbox::open(&directory, config.max_inbox_messages)?.import_event_page(page)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn private_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::create_dir_all(path)?;
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "session path must be a real directory"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn regular_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "session data must be a regular file"
    );
    Ok(())
}

pub(crate) fn private_file(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => regular_file(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(path)?;
        }
        Err(error) => return Err(error.into()),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn persist_metadata(directory: &Path, metadata: &Metadata) -> Result<()> {
    let path = directory.join("metadata.json");
    let temporary = directory.join("metadata.tmp");
    if path.exists() {
        regular_file(&path)?;
    }
    // The lifecycle/state lock owns this name; reject symlinks before truncation.
    private_file(&temporary)?;
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&temporary)?;
    serde_json::to_writer(&mut file, metadata)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(&temporary, &path)?;
    #[cfg(unix)]
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

fn clear_credentials(directory: &Path) -> Result<()> {
    for name in ["credentials.db", "credentials.db-wal", "credentials.db-shm"] {
        let path = directory.join(name);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                regular_file(&path)?;
                fs::remove_file(&path)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SessionRequest {
    session_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyRequest {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PairCodeRequest {
    session_id: String,
    phone_number: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EventsRequest {
    session_id: String,
    #[serde(default)]
    after_sequence: u64,
    #[serde(default = "default_event_limit")]
    limit: usize,
}
fn default_event_limit() -> usize {
    100
}

pub fn register(registry: &mut Registry) {
    registry.add::<SessionRequest, SessionSummary, _, _>(
        "whatsapp::sessions::create",
        "Create persistent session metadata without connecting to WhatsApp",
        |manager, request| async move { manager.create(&request.session_id).await },
    );
    registry.add::<SessionRequest, SessionSummary, _, _>(
        "whatsapp::sessions::start",
        "Start a native session and return its current connection state",
        |manager, request| async move { manager.start(&request.session_id).await },
    );
    registry.add::<SessionRequest, SessionSummary, _, _>(
        "whatsapp::sessions::stop",
        "Stop a native session while preserving credentials",
        |manager, request| async move { manager.stop(&request.session_id).await },
    );
    registry.add::<SessionRequest, SessionSummary, _, _>(
        "whatsapp::sessions::get",
        "Get a session summary without credentials or pairing secrets",
        |manager, request| async move { manager.get(&request.session_id).await },
    );
    registry.add::<EmptyRequest, Vec<SessionSummary>, _, _>(
        "whatsapp::sessions::list",
        "List persistent sessions without connecting",
        |manager, _| async move { Ok(manager.list().await) },
    );
    registry.add::<SessionRequest, Option<PairingCode>, _, _>(
        "whatsapp::sessions::qr",
        "Read the current unexpired pairing QR payload; null when unavailable",
        |manager, request| async move { manager.qr(&request.session_id).await },
    );
    registry.add::<PairCodeRequest, PairingCode, _, _>(
        "whatsapp::sessions::pair_code",
        "Request a native phone-number pairing code for a started session",
        |manager, request| async move {
            manager
                .pair_code(&request.session_id, &request.phone_number)
                .await
        },
    );
    registry.add::<SessionRequest, SessionSummary, _, _>(
        "whatsapp::sessions::logout",
        "Request native best-effort device unlink, stop, and clear local credentials",
        |manager, request| async move { manager.logout(&request.session_id).await },
    );
    registry.add::<SessionRequest, SessionSummary, _, _>(
        "whatsapp::sessions::delete",
        "Stop and remove one session and its local credentials",
        |manager, request| async move { manager.delete(&request.session_id).await },
    );
    registry.add::<EventsRequest, EventPage, _, _>(
        "whatsapp::events::poll",
        "Poll bounded session events after a sequence cursor, with retention-gap reporting",
        |manager, request| async move {
            manager
                .events(&request.session_id, request.after_sequence, request.limit)
                .await
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use whatsapp_rust::prelude::{BatchOrigin, InboundMessage, MessageBatch, MessageInfo, wa};
    use whatsapp_rust::types::events::{Connected, PairingQrCode};

    async fn fixture() -> (TempDir, Arc<SessionManager>, Arc<Session>, NativeEvents) {
        let directory = TempDir::new().unwrap();
        let manager = SessionManager::open(Config {
            data_dir: directory.path().join("sessions"),
            shutdown_timeout_secs: 1,
            ..Config::default()
        })
        .unwrap();
        manager.create("work").await.unwrap();
        let session = manager.session("work").unwrap();
        let handler = NativeEvents {
            session: Arc::downgrade(&session),
            generation: 0,
        };
        (directory, manager, session, handler)
    }

    /// Real SQLite and native client initialization, with the socket run loop parked.
    /// The task still follows the native shutdown signal, so teardown is exercised offline.
    async fn parked_native(session: &Arc<Session>) -> Weak<Client> {
        let path = session.directory.join("credentials.db");
        private_file(&path).unwrap();
        let store = Arc::new(SqliteStore::new(path.to_str().unwrap()).await.unwrap());
        let bot = Bot::builder()
            .with_backend_arc(store.clone())
            .build()
            .await
            .unwrap();
        let client = bot.client();
        let weak = Arc::downgrade(&client);
        let shutdown = client.shutdown_signal();
        let task = tokio::spawn(async move {
            wacore::runtime::wait_for_shutdown(&shutdown).await;
            drop(bot);
        });
        session.lifecycle.lock().await.running = Some(Running {
            client,
            store: Arc::downgrade(&store),
            task,
        });
        session.state.lock().unwrap().status = SessionStatus::Starting;
        weak
    }

    #[tokio::test]
    async fn duplicate_start_keeps_one_native_client_and_stop_preserves_sqlite() {
        let (_directory, manager, session, _handler) = fixture().await;
        let client = parked_native(&session).await;
        let (left, right) = tokio::join!(manager.start("work"), manager.start("work"));
        assert_eq!(left.unwrap().status, SessionStatus::Starting);
        assert_eq!(right.unwrap().status, SessionStatus::Starting);
        {
            let lifecycle = session.lifecycle.lock().await;
            assert!(Weak::ptr_eq(
                &client,
                &Arc::downgrade(&lifecycle.running.as_ref().unwrap().client)
            ));
        }
        let (left, right) = tokio::join!(manager.stop("work"), manager.stop("work"));
        assert_eq!(left.unwrap().status, SessionStatus::Stopped);
        assert_eq!(right.unwrap().status, SessionStatus::Stopped);
        assert!(client.upgrade().is_none());
        assert!(session.directory.join("credentials.db").is_file());
    }

    #[tokio::test]
    async fn delete_waits_for_operation_client_before_removing_sqlite() {
        let (_directory, manager, session, _handler) = fixture().await;
        let client = parked_native(&session).await.upgrade().unwrap();
        assert!(manager.delete("work").await.is_err());
        assert!(session.directory.join("credentials.db").is_file());
        assert!(!session.state.lock().unwrap().deleted);
        drop(client);
        manager.delete("work").await.unwrap();
        assert!(!session.directory.exists());
        assert!(manager.get("work").await.is_err());
    }

    #[tokio::test]
    async fn shutdown_releases_native_clients_without_erasing_credentials() {
        let (_directory, manager, session, _handler) = fixture().await;
        let client = parked_native(&session).await;
        manager.shutdown().await;
        assert!(client.upgrade().is_none());
        assert!(session.directory.join("credentials.db").is_file());
        assert_eq!(
            manager.get("work").await.unwrap().status,
            SessionStatus::Stopped
        );
    }

    #[tokio::test]
    async fn stopping_invalidates_pairing_and_callbacks_even_without_a_live_handle() {
        let (_directory, manager, _session, handler) = fixture().await;
        handler.handle_event(Arc::new(NativeEvent::PairingQrCode(
            PairingQrCode::builder()
                .code("private QR".into())
                .timeout(Duration::from_secs(60))
                .build(),
        )));
        assert!(manager.qr("work").await.unwrap().is_some());
        manager.stop("work").await.unwrap();
        handler.handle_event(Arc::new(NativeEvent::Connected(
            Connected::builder().build(),
        )));
        assert_eq!(
            manager.get("work").await.unwrap().status,
            SessionStatus::Stopped
        );
        assert!(manager.qr("work").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn pairing_qr_expires_and_secrets_never_enter_event_payloads() {
        let (_directory, manager, _session, handler) = fixture().await;
        handler.handle_event(Arc::new(NativeEvent::PairingQrCode(
            PairingQrCode::builder()
                .code("private QR".into())
                .timeout(Duration::from_secs(60))
                .build(),
        )));
        assert_eq!(
            manager.qr("work").await.unwrap().unwrap().code,
            "private QR"
        );
        let events = manager.events("work", 0, 100).await.unwrap();
        let encoded = serde_json::to_string(&events).unwrap();
        assert!(!encoded.contains("private QR"));
        handler.handle_event(Arc::new(NativeEvent::PairingQrCode(
            PairingQrCode::builder()
                .code("expired QR".into())
                .timeout(Duration::ZERO)
                .build(),
        )));
        assert!(manager.qr("work").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn native_inbound_message_retains_structured_body_and_sender() {
        let (_directory, manager, _session, handler) = fixture().await;
        let mut info = MessageInfo {
            id: "native-message-1".into(),
            ..Default::default()
        };
        info.source.chat = "15551234567@s.whatsapp.net".parse().unwrap();
        info.source.sender = info.source.chat.clone();
        let inbound = InboundMessage::builder()
            .message(Arc::new(wa::Message {
                conversation: Some("hello".to_owned()),
                ..Default::default()
            }))
            .info(Arc::new(info))
            .build();
        handler.handle_event(Arc::new(NativeEvent::Messages(
            MessageBatch::builder()
                .messages(Arc::from([inbound]))
                .origin(BatchOrigin::Live)
                .build(),
        )));
        let page = manager.events("work", 0, 10).await.unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].kind, "message.received");
        assert_eq!(page.events[0].data["message"]["conversation"], "hello");
        assert_eq!(page.events[0].data["info"]["id"], "native-message-1");
        assert!(page.events[0].data["info"]["source"]["sender"].is_string());
        assert!(manager.client("work").await.is_err());
    }

    #[tokio::test]
    async fn incoming_call_exposes_routing_ids_and_media_flags_without_raw_payload() {
        use whatsapp_rust::types::call::{CallAction, IncomingCall};
        let (_directory, manager, _session, handler) = fixture().await;
        let peer = "15551234567:3@s.whatsapp.net".parse().unwrap();
        let creator = "15551234567@s.whatsapp.net".parse().unwrap();
        let action = CallAction::Offer {
            call_id: "call-123".to_owned(),
            call_creator: creator,
            caller_pn: None,
            caller_country_code: None,
            device_class: None,
            joinable: false,
            is_video: true,
            audio: Vec::new(),
            group_jid: None,
        };
        let call =
            IncomingCall::new_for_test(peer, "stanza-123".into(), Default::default(), action);
        handler.handle_event(Arc::new(NativeEvent::IncomingCall(Box::new(call))));
        let page = manager.events("work", 0, 10).await.unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].kind, "call.incoming");
        assert_eq!(page.events[0].data["call_id"], "call-123");
        assert_eq!(page.events[0].data["peer"], "15551234567:3@s.whatsapp.net");
        assert_eq!(
            page.events[0].data["call_creator"],
            "15551234567@s.whatsapp.net"
        );
        assert_eq!(page.events[0].data["is_video"], true);
        assert!(page.events[0].data.get("raw").is_none());
        assert!(page.events[0].data.get("media").is_none());
    }

    #[tokio::test]
    async fn cached_pair_code_is_not_reused_without_matching_phone_request() {
        let (_directory, manager, session, _handler) = fixture().await;
        {
            let mut state = session.state.lock().unwrap();
            state.status = SessionStatus::Pairing;
            state.pair_code = Some(PairingCode {
                code: "ABC12345".to_owned(),
                expires_at_ms: now_ms() + 60_000,
            });
        }
        assert!(manager.pair_code("work", "15550000002").await.is_err());
    }

    #[tokio::test]
    async fn shutdown_timeout_also_bounds_waiting_for_an_active_lifecycle_request() {
        let (_directory, manager, session, _handler) = fixture().await;
        let _request = session.lifecycle.lock().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(1500), manager.shutdown())
                .await
                .is_ok()
        );
    }
}
