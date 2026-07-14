//! All domain logic. Every mutation returns the response **and** the
//! events to emit ([`EmittableEvent`]), so emission stays at the edge
//! (the function handlers) and the service is fully testable without an
//! engine.
//!
//! Concurrency: the worker is the single writer of its state scopes;
//! mutations are serialized **per session** with an async lock so
//! read-modify-write invariants hold (active leaf, `message_count`,
//! `revision`, idempotent append). Reads take no lock.
//!
//! Determinism hooks: id generation and the clock are injected so tests
//! can assert exact ids and timestamps.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::config::WorkerConfig;
use crate::configuration::ConfigCell;
use crate::error::SessionError;
use crate::events::{
    EmittableEvent, MessageAddedEvent, MessageUpdatedEvent, MetaUpdatedEvent, SessionCreatedEvent,
    SessionDeletedEvent, SessionEvent, StatusChangedEvent,
};
use crate::functions::append::{AppendRequest, AppendResponse};
use crate::functions::append_many::{AppendManyRequest, AppendManyResponse};
use crate::functions::create::{CreateRequest, CreateResponse};
use crate::functions::delete::{DeleteRequest, DeleteResponse};
use crate::functions::ensure::{EnsureRequest, EnsureResponse};
use crate::functions::fork::{ForkRequest, ForkResponse};
use crate::functions::get::{GetRequest, GetResponse};
use crate::functions::get_message::{GetMessageRequest, GetMessageResponse};
use crate::functions::list::{ListOrder, ListRequest, ListResponse};
use crate::functions::messages::{MessageItem, MessagesRequest, MessagesResponse};
use crate::functions::set_active_leaf::{SetActiveLeafRequest, SetActiveLeafResponse};
use crate::functions::set_draft::{SetDraftRequest, SetDraftResponse};
use crate::functions::set_meta::{SetMetaRequest, SetMetaResponse};
use crate::functions::set_status::{SetStatusRequest, SetStatusResponse};
use crate::functions::update_message::{UpdateMessageRequest, UpdateMessageResponse};
use crate::store::SessionStore;
use crate::types::{
    metadata_matches, AgentMessage, CustomPayload, SessionEntry, SessionMeta, SessionStatus,
};

/// Id generation, injected for deterministic tests.
pub trait IdGen: Send + Sync {
    fn session_id(&self) -> String;
    fn entry_id(&self) -> String;
}

/// Production ids: `s_<uuid>` / `e_<uuid>`.
pub struct UuidIds;

impl IdGen for UuidIds {
    fn session_id(&self) -> String {
        format!("s_{}", Uuid::new_v4().simple())
    }

    fn entry_id(&self) -> String {
        format!("e_{}", Uuid::new_v4().simple())
    }
}

/// Time source, injected for deterministic tests.
pub trait Clock: Send + Sync {
    /// Milliseconds since epoch.
    fn now_ms(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        // Fail fast on a pre-1970 system clock: a 0 fallback would
        // persist bogus epoch timestamps into a durable store.
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is before UNIX_EPOCH")
            .as_millis() as i64
    }
}

type ServiceResult<T> = Result<(T, Vec<EmittableEvent>), SessionError>;

/// Wrap a config value in a fresh, standalone [`ConfigCell`] (not shared with
/// a configuration-change trigger). Used by the non-production constructors.
fn new_config_cell(cfg: &WorkerConfig) -> ConfigCell {
    Arc::new(RwLock::new(Arc::new(cfg.clone())))
}

pub struct SessionService {
    store: Arc<dyn SessionStore>,
    ids: Arc<dyn IdGen>,
    clock: Arc<dyn Clock>,
    /// Hot-swappable config snapshot shared with the configuration-change
    /// trigger; `list` / `messages` read the current list limits per call so
    /// a `configuration::set` of the limits applies without a restart.
    config: ConfigCell,
    /// Per-session mutation locks. Entries are kept for the worker
    /// lifetime (tens of bytes per touched session) — never removed, so
    /// two waiters can never end up serialized on different locks.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl SessionService {
    pub fn new(store: Arc<dyn SessionStore>, cfg: &WorkerConfig) -> Self {
        Self::with_config_cell(store, new_config_cell(cfg))
    }

    /// Production constructor: shares the hot-swappable [`ConfigCell`] with the
    /// configuration-change trigger, so a live `configuration::set` of the
    /// list limits is picked up without a restart.
    pub fn with_config_cell(store: Arc<dyn SessionStore>, config: ConfigCell) -> Self {
        Self::with_parts_cell(store, Arc::new(UuidIds), Arc::new(SystemClock), config)
    }

    /// Full-injection constructor used by tests. The config is wrapped in a
    /// private cell (tests don't exercise hot-reload).
    pub fn with_parts(
        store: Arc<dyn SessionStore>,
        ids: Arc<dyn IdGen>,
        clock: Arc<dyn Clock>,
        cfg: &WorkerConfig,
    ) -> Self {
        Self::with_parts_cell(store, ids, clock, new_config_cell(cfg))
    }

    fn with_parts_cell(
        store: Arc<dyn SessionStore>,
        ids: Arc<dyn IdGen>,
        clock: Arc<dyn Clock>,
        config: ConfigCell,
    ) -> Self {
        Self {
            store,
            ids,
            clock,
            config,
            locks: Mutex::new(HashMap::new()),
        }
    }

    async fn lock_session(&self, session_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut map = self
                .locks
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            map.entry(session_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        lock.lock_owned().await
    }

    async fn meta_or_not_found(&self, session_id: &str) -> Result<SessionMeta, SessionError> {
        self.store
            .get_meta(session_id)
            .await?
            .ok_or_else(|| SessionError::NotFound(format!("session {session_id} does not exist")))
    }

    async fn clamp_limit(&self, limit: Option<usize>) -> usize {
        let cfg = self.config.read().await;
        limit
            .unwrap_or(cfg.default_list_limit)
            .clamp(1, cfg.max_list_limit)
    }

    // -----------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------

    pub async fn create(&self, req: CreateRequest) -> ServiceResult<CreateResponse> {
        let now = self.clock.now_ms();
        let meta = SessionMeta {
            session_id: self.ids.session_id(),
            title: req.title.unwrap_or_default(),
            description: req.description.unwrap_or_default(),
            status: SessionStatus::Idle,
            status_reason: None,
            metadata: req.metadata,
            forked_from: None,
            draft: None,
            created_at: now,
            updated_at: now,
            message_count: 0,
        };
        self.store.put_meta(&meta).await?;
        let event = created_event(&meta);
        Ok((
            CreateResponse {
                session_id: meta.session_id.clone(),
                meta,
            },
            vec![event],
        ))
    }

    pub async fn ensure(&self, req: EnsureRequest) -> ServiceResult<EnsureResponse> {
        if req.session_id.is_empty() {
            return Err(SessionError::InvalidRequest(
                "session_id must not be empty".into(),
            ));
        }
        let _guard = self.lock_session(&req.session_id).await;

        if let Some(meta) = self.store.get_meta(&req.session_id).await? {
            return Ok((
                EnsureResponse {
                    session_id: meta.session_id.clone(),
                    meta,
                    created: false,
                },
                vec![],
            ));
        }

        let now = self.clock.now_ms();
        let meta = SessionMeta {
            session_id: req.session_id.clone(),
            title: req.title.unwrap_or_default(),
            description: req.description.unwrap_or_default(),
            status: SessionStatus::Idle,
            status_reason: None,
            metadata: req.metadata,
            forked_from: None,
            draft: None,
            created_at: now,
            updated_at: now,
            message_count: 0,
        };
        self.store.put_meta(&meta).await?;
        let event = created_event(&meta);
        Ok((
            EnsureResponse {
                session_id: meta.session_id.clone(),
                meta,
                created: true,
            },
            vec![event],
        ))
    }

    pub async fn get(&self, req: GetRequest) -> Result<Option<GetResponse>, SessionError> {
        Ok(self
            .store
            .get_meta(&req.session_id)
            .await?
            .map(|meta| GetResponse { meta }))
    }

    pub async fn list(&self, req: ListRequest) -> ServiceResult<ListResponse> {
        let order = req.order.unwrap_or_default();
        let limit = self.clamp_limit(req.limit).await;

        let mut metas = self.store.list_metas().await?;
        if let Some(status) = req.status {
            metas.retain(|m| m.status == status);
        }
        if let Some(want) = &req.metadata {
            metas.retain(|m| metadata_matches(want, m.metadata.as_ref()));
        }
        metas.sort_by(|a, b| cmp_in_order(sort_key(a, order), sort_key(b, order), order));

        let start = match &req.cursor {
            None => 0,
            Some(raw) => {
                let cursor: SessionsCursor = decode_cursor(raw)?;
                if cursor.o != order_tag(order) {
                    return Err(SessionError::InvalidCursor(format!(
                        "cursor was issued for order {} but the request asks for {}",
                        cursor.o,
                        order_tag(order)
                    )));
                }
                let cursor_key = (cursor.k, cursor.id);
                metas.partition_point(|m| {
                    let key = sort_key(m, order);
                    cmp_in_order((key.0, key.1.to_string()), cursor_key.clone(), order)
                        != std::cmp::Ordering::Greater
                })
            }
        };

        let end = (start + limit).min(metas.len());
        let page: Vec<SessionMeta> = metas[start..end].to_vec();
        let next_cursor = if end < metas.len() {
            page.last().map(|m| {
                let key = sort_key(m, order);
                encode_cursor(&SessionsCursor {
                    o: order_tag(order).to_string(),
                    k: key.0,
                    id: key.1.to_string(),
                })
            })
        } else {
            None
        };

        Ok((
            ListResponse {
                sessions: page,
                next_cursor,
            },
            vec![],
        ))
    }

    pub async fn set_meta(&self, req: SetMetaRequest) -> ServiceResult<SetMetaResponse> {
        let _guard = self.lock_session(&req.session_id).await;
        let mut meta = self.meta_or_not_found(&req.session_id).await?;

        if req.title.is_none() && req.description.is_none() && req.metadata.is_none() {
            return Ok((SetMetaResponse { meta }, vec![]));
        }

        if let Some(title) = req.title {
            meta.title = title;
        }
        if let Some(description) = req.description {
            meta.description = description;
        }
        if let Some(metadata) = req.metadata {
            // A supplied metadata object replaces the stored one.
            meta.metadata = Some(metadata);
        }
        let now = self.clock.now_ms();
        meta.updated_at = now;
        self.store.put_meta(&meta).await?;

        let event = EmittableEvent {
            event: SessionEvent::MetaUpdated(MetaUpdatedEvent {
                session_id: meta.session_id.clone(),
                title: meta.title.clone(),
                description: meta.description.clone(),
                metadata: meta.metadata.clone(),
                timestamp: now,
            }),
            session_metadata: meta.metadata.clone(),
        };
        Ok((SetMetaResponse { meta }, vec![event]))
    }

    pub async fn set_draft(&self, req: SetDraftRequest) -> ServiceResult<SetDraftResponse> {
        let _guard = self.lock_session(&req.session_id).await;
        let mut meta = self.meta_or_not_found(&req.session_id).await?;

        // Whitespace-only input is "nothing worth keeping" — normalize to a
        // cleared draft so an emptied composer removes the stored record.
        let draft = req.draft.filter(|d| !d.trim().is_empty());
        if meta.draft == draft {
            return Ok((SetDraftResponse { draft: meta.draft }, vec![]));
        }

        // Deliberately no `updated_at` bump and no event: drafts are saved at
        // keystroke cadence, and a save must neither re-order `session::list`
        // nor spam meta-updated subscribers. Consumers read the draft back
        // from `session::get` / `session::list`.
        meta.draft = draft;
        self.store.put_meta(&meta).await?;
        Ok((SetDraftResponse { draft: meta.draft }, vec![]))
    }

    pub async fn set_status(&self, req: SetStatusRequest) -> ServiceResult<SetStatusResponse> {
        let _guard = self.lock_session(&req.session_id).await;
        let mut meta = self.meta_or_not_found(&req.session_id).await?;

        // Reason is retained while `working` (live phase detail, e.g.
        // "waiting for <model>") and on `error` (failure cause);
        // idle/done always clear it.
        let new_reason = match req.status {
            SessionStatus::Working | SessionStatus::Error => req.reason,
            _ => None,
        };

        // Spec-strict no-op: same status AND same stored reason fires no
        // event. A reason change alone re-emits so UIs can render live
        // phase updates within one `working` stretch.
        if meta.status == req.status && meta.status_reason == new_reason {
            return Ok((
                SetStatusResponse {
                    status: meta.status,
                    previous_status: meta.status,
                },
                vec![],
            ));
        }

        let previous_status = meta.status;
        meta.status = req.status;
        meta.status_reason = new_reason;
        let now = self.clock.now_ms();
        meta.updated_at = now;
        self.store.put_meta(&meta).await?;

        let event = EmittableEvent {
            event: SessionEvent::StatusChanged(StatusChangedEvent {
                session_id: meta.session_id.clone(),
                status: meta.status,
                previous_status,
                status_reason: meta.status_reason.clone(),
                timestamp: now,
            }),
            session_metadata: meta.metadata.clone(),
        };
        Ok((
            SetStatusResponse {
                status: meta.status,
                previous_status,
            },
            vec![event],
        ))
    }

    pub async fn delete(&self, req: DeleteRequest) -> ServiceResult<DeleteResponse> {
        let _guard = self.lock_session(&req.session_id).await;

        let Some(meta) = self.store.get_meta(&req.session_id).await? else {
            return Ok((DeleteResponse { deleted: false }, vec![]));
        };

        self.store.delete_entries(&req.session_id).await?;
        self.store.delete_active_leaf(&req.session_id).await?;
        self.store.delete_meta(&req.session_id).await?;

        let event = EmittableEvent {
            event: SessionEvent::Deleted(SessionDeletedEvent {
                session_id: req.session_id.clone(),
                timestamp: self.clock.now_ms(),
            }),
            // Filters evaluate against the metadata as-of-deletion.
            session_metadata: meta.metadata,
        };
        Ok((DeleteResponse { deleted: true }, vec![event]))
    }

    // -----------------------------------------------------------------
    // Messages
    // -----------------------------------------------------------------

    pub async fn append(&self, req: AppendRequest) -> ServiceResult<AppendResponse> {
        let _guard = self.lock_session(&req.session_id).await;
        let mut meta = self.meta_or_not_found(&req.session_id).await?;

        enum Body {
            Message(Box<AgentMessage>),
            Custom(CustomPayload),
        }
        let body = match (req.message, req.custom) {
            (Some(m), None) => Body::Message(Box::new(m)),
            (None, Some(c)) => Body::Custom(c),
            _ => {
                return Err(SessionError::InvalidRequest(
                    "exactly one of `message` / `custom` must be supplied".into(),
                ))
            }
        };

        // Idempotent on entry_id: appending an id that already exists is
        // a no-op — the existing entry is returned and no event fires.
        if let Some(entry_id) = &req.entry_id {
            if let Some(existing) = self.store.get_entry(&req.session_id, entry_id).await? {
                return Ok((
                    AppendResponse {
                        entry_id: existing.id().to_string(),
                        parent_id: existing.parent_id().map(str::to_string),
                        timestamp: existing.timestamp(),
                    },
                    vec![],
                ));
            }
        }

        let parent_id = match &req.parent_id {
            Some(parent) => {
                if self
                    .store
                    .get_entry(&req.session_id, parent)
                    .await?
                    .is_none()
                {
                    return Err(SessionError::ParentNotFound(format!(
                        "parent entry {parent} does not exist in session {}",
                        req.session_id
                    )));
                }
                Some(parent.clone())
            }
            None => self.store.get_active_leaf(&req.session_id).await?,
        };

        let entry_id = req.entry_id.unwrap_or_else(|| self.ids.entry_id());
        let now = self.clock.now_ms();
        let is_message = matches!(body, Body::Message(_));
        let (entry, event_message, event_custom) = match body {
            Body::Message(message) => (
                SessionEntry::Message {
                    id: entry_id.clone(),
                    parent_id: parent_id.clone(),
                    timestamp: now,
                    revision: 0,
                    origin: req.origin.clone(),
                    message: (*message).clone(),
                },
                Some(*message),
                None,
            ),
            Body::Custom(custom) => (
                SessionEntry::Custom {
                    id: entry_id.clone(),
                    parent_id: parent_id.clone(),
                    timestamp: now,
                    revision: 0,
                    origin: req.origin.clone(),
                    custom_type: custom.custom_type.clone(),
                    data: custom.data.clone(),
                },
                None,
                Some(custom),
            ),
        };

        self.store.put_entry(&req.session_id, &entry).await?;
        // Appending always moves the active leaf to the new entry.
        self.store
            .set_active_leaf(&req.session_id, &entry_id)
            .await?;
        if is_message {
            meta.message_count += 1;
        }
        meta.updated_at = now;
        self.store.put_meta(&meta).await?;

        let event = EmittableEvent {
            event: SessionEvent::MessageAdded(MessageAddedEvent {
                session_id: req.session_id.clone(),
                entry_id: entry_id.clone(),
                parent_id: parent_id.clone(),
                message: event_message,
                custom: event_custom,
                origin: req.origin,
                timestamp: now,
            }),
            session_metadata: meta.metadata.clone(),
        };
        Ok((
            AppendResponse {
                entry_id,
                parent_id,
                timestamp: now,
            },
            vec![event],
        ))
    }

    pub async fn append_many(&self, req: AppendManyRequest) -> ServiceResult<AppendManyResponse> {
        let _guard = self.lock_session(&req.session_id).await;
        let mut meta = self.meta_or_not_found(&req.session_id).await?;

        if req.messages.is_empty() {
            return Err(SessionError::EmptyBatch(
                "messages must contain at least one message".into(),
            ));
        }

        let mut parent_id = match &req.parent_id {
            Some(parent) => {
                if self
                    .store
                    .get_entry(&req.session_id, parent)
                    .await?
                    .is_none()
                {
                    return Err(SessionError::ParentNotFound(format!(
                        "parent entry {parent} does not exist in session {}",
                        req.session_id
                    )));
                }
                Some(parent.clone())
            }
            None => self.store.get_active_leaf(&req.session_id).await?,
        };

        let now = self.clock.now_ms();
        let mut entry_ids = Vec::with_capacity(req.messages.len());
        let mut events = Vec::with_capacity(req.messages.len());

        for message in req.messages {
            let entry_id = self.ids.entry_id();
            let entry = SessionEntry::Message {
                id: entry_id.clone(),
                parent_id: parent_id.clone(),
                timestamp: now,
                revision: 0,
                origin: req.origin.clone(),
                message: message.clone(),
            };
            self.store.put_entry(&req.session_id, &entry).await?;

            events.push(EmittableEvent {
                event: SessionEvent::MessageAdded(MessageAddedEvent {
                    session_id: req.session_id.clone(),
                    entry_id: entry_id.clone(),
                    parent_id: parent_id.clone(),
                    message: Some(message),
                    custom: None,
                    origin: req.origin.clone(),
                    timestamp: now,
                }),
                session_metadata: meta.metadata.clone(),
            });

            parent_id = Some(entry_id.clone());
            entry_ids.push(entry_id);
        }

        let last_entry_id = entry_ids
            .last()
            .expect("non-empty batch always has a last entry")
            .clone();
        self.store
            .set_active_leaf(&req.session_id, &last_entry_id)
            .await?;
        meta.message_count += entry_ids.len() as u64;
        meta.updated_at = now;
        self.store.put_meta(&meta).await?;

        Ok((
            AppendManyResponse {
                entry_ids,
                last_entry_id,
            },
            events,
        ))
    }

    pub async fn update_message(
        &self,
        req: UpdateMessageRequest,
    ) -> ServiceResult<UpdateMessageResponse> {
        let _guard = self.lock_session(&req.session_id).await;
        let mut meta = self.meta_or_not_found(&req.session_id).await?;

        let entry = self
            .store
            .get_entry(&req.session_id, &req.entry_id)
            .await?
            .ok_or_else(|| {
                SessionError::EntryNotFound(format!(
                    "entry {} does not exist in session {}",
                    req.entry_id, req.session_id
                ))
            })?;

        let SessionEntry::Message {
            id,
            parent_id,
            timestamp,
            revision,
            origin,
            mut message,
        } = entry
        else {
            return Err(SessionError::InvalidEntryKind(format!(
                "entry {} is a custom entry; session::update-message only applies to messages",
                req.entry_id
            )));
        };

        // Optimistic concurrency: on mismatch nothing is written and the
        // current revision is returned.
        if let Some(expected) = req.expected_revision {
            if expected != revision {
                return Ok((
                    UpdateMessageResponse {
                        updated: false,
                        revision,
                    },
                    vec![],
                ));
            }
        }

        message.set_content(req.content);
        if let Some(new_details) = req.details {
            match &mut message {
                AgentMessage::FunctionResult { details, .. } => *details = new_details,
                AgentMessage::Custom { details, .. } => *details = Some(new_details),
                _ => {
                    return Err(SessionError::DetailsNotSupported(format!(
                        "entry {} has role {:?}; `details` applies to function_result and custom messages",
                        req.entry_id,
                        message.role()
                    )))
                }
            }
        }

        let new_revision = revision + 1;
        let updated_entry = SessionEntry::Message {
            id,
            parent_id,
            timestamp,
            revision: new_revision,
            origin,
            message: message.clone(),
        };
        self.store
            .put_entry(&req.session_id, &updated_entry)
            .await?;

        let now = self.clock.now_ms();
        meta.updated_at = now;
        self.store.put_meta(&meta).await?;

        let event = EmittableEvent {
            event: SessionEvent::MessageUpdated(MessageUpdatedEvent {
                session_id: req.session_id.clone(),
                entry_id: req.entry_id.clone(),
                message,
                revision: new_revision,
                origin: req.origin,
                timestamp: now,
            }),
            session_metadata: meta.metadata.clone(),
        };
        Ok((
            UpdateMessageResponse {
                updated: true,
                revision: new_revision,
            },
            vec![event],
        ))
    }

    pub async fn messages(&self, req: MessagesRequest) -> ServiceResult<MessagesResponse> {
        self.meta_or_not_found(&req.session_id).await?;

        let entries = self.store.list_entries(&req.session_id).await?;
        let by_id: HashMap<&str, &SessionEntry> = entries.iter().map(|e| (e.id(), e)).collect();

        let leaf = match &req.from_entry_id {
            Some(from) => {
                if !by_id.contains_key(from.as_str()) {
                    return Err(SessionError::EntryNotFound(format!(
                        "entry {from} does not exist in session {}",
                        req.session_id
                    )));
                }
                Some(from.clone())
            }
            None => self.store.get_active_leaf(&req.session_id).await?,
        };

        let Some(leaf) = leaf else {
            return Ok((
                MessagesResponse {
                    messages: vec![],
                    next_cursor: None,
                },
                vec![],
            ));
        };

        let path = active_path(&by_id, &leaf)?;

        let include_custom = req.include_custom.unwrap_or(false);
        let filtered: Vec<&SessionEntry> = path
            .into_iter()
            .filter(|entry| match entry {
                SessionEntry::Message { message, .. } => match &req.roles {
                    Some(roles) => roles.contains(&message.role()),
                    None => true,
                },
                // A `roles` filter is an explicit narrowing to message
                // roles, so it also drops custom entries.
                SessionEntry::Custom { .. } => include_custom && req.roles.is_none(),
            })
            .collect();

        let start = match &req.cursor {
            None => 0,
            Some(raw) => {
                let cursor: MessagesCursor = decode_cursor(raw)?;
                let position = filtered
                    .iter()
                    .position(|e| e.id() == cursor.id)
                    .ok_or_else(|| {
                        SessionError::InvalidCursor(format!(
                            "cursor entry {} is not on the requested path",
                            cursor.id
                        ))
                    })?;
                position + 1
            }
        };

        let limit = self.clamp_limit(req.limit).await;
        let end = (start + limit).min(filtered.len());
        let page = &filtered[start..end];
        let next_cursor = if end < filtered.len() {
            page.last().map(|e| {
                encode_cursor(&MessagesCursor {
                    id: e.id().to_string(),
                })
            })
        } else {
            None
        };

        let items = page
            .iter()
            .map(|entry| match entry {
                SessionEntry::Message { id, message, .. } => MessageItem {
                    entry_id: id.clone(),
                    message: Some(message.clone()),
                    custom: None,
                },
                SessionEntry::Custom {
                    id,
                    custom_type,
                    data,
                    ..
                } => MessageItem {
                    entry_id: id.clone(),
                    message: None,
                    custom: Some(CustomPayload {
                        custom_type: custom_type.clone(),
                        data: data.clone(),
                    }),
                },
            })
            .collect();

        Ok((
            MessagesResponse {
                messages: items,
                next_cursor,
            },
            vec![],
        ))
    }

    pub async fn get_message(
        &self,
        req: GetMessageRequest,
    ) -> Result<Option<GetMessageResponse>, SessionError> {
        Ok(self
            .store
            .get_entry(&req.session_id, &req.entry_id)
            .await?
            .map(|entry| GetMessageResponse { entry }))
    }

    // -----------------------------------------------------------------
    // Branching
    // -----------------------------------------------------------------

    pub async fn fork(&self, req: ForkRequest) -> ServiceResult<ForkResponse> {
        let _guard = self.lock_session(&req.session_id).await;
        let source = self.meta_or_not_found(&req.session_id).await?;

        let entries = self.store.list_entries(&req.session_id).await?;
        let by_id: HashMap<&str, &SessionEntry> = entries.iter().map(|e| (e.id(), e)).collect();
        if !by_id.contains_key(req.entry_id.as_str()) {
            return Err(SessionError::EntryNotFound(format!(
                "entry {} does not exist in session {}",
                req.entry_id, req.session_id
            )));
        }

        // Copy-on-fork: copy every entry on the root -> entry_id path
        // with fresh ids; the parent chain is preserved structurally.
        let path = active_path(&by_id, &req.entry_id)?;

        let new_session_id = self.ids.session_id();
        let now = self.clock.now_ms();
        let mut id_map: HashMap<String, String> = HashMap::new();
        let mut copies = Vec::with_capacity(path.len());
        let mut message_count: u64 = 0;

        for entry in &path {
            let new_id = self.ids.entry_id();
            id_map.insert(entry.id().to_string(), new_id.clone());
            let new_parent = entry.parent_id().map(|p| {
                id_map
                    .get(p)
                    .cloned()
                    .expect("parents precede children on the path")
            });

            let copy = match entry {
                SessionEntry::Message {
                    timestamp,
                    origin,
                    message,
                    ..
                } => {
                    message_count += 1;
                    SessionEntry::Message {
                        id: new_id,
                        parent_id: new_parent,
                        timestamp: *timestamp,
                        // Fresh entries start a fresh revision space.
                        revision: 0,
                        origin: origin.clone(),
                        message: message.clone(),
                    }
                }
                SessionEntry::Custom {
                    timestamp,
                    origin,
                    custom_type,
                    data,
                    ..
                } => SessionEntry::Custom {
                    id: new_id,
                    parent_id: new_parent,
                    timestamp: *timestamp,
                    revision: 0,
                    origin: origin.clone(),
                    custom_type: custom_type.clone(),
                    data: data.clone(),
                },
            };
            copies.push(copy);
        }

        for copy in &copies {
            self.store.put_entry(&new_session_id, copy).await?;
        }

        let new_leaf = id_map
            .get(&req.entry_id)
            .expect("fork point is on the path")
            .clone();
        self.store
            .set_active_leaf(&new_session_id, &new_leaf)
            .await?;

        let meta = SessionMeta {
            session_id: new_session_id.clone(),
            title: req.title.unwrap_or_else(|| source.title.clone()),
            description: source.description.clone(),
            status: SessionStatus::Idle,
            status_reason: None,
            // Tenancy propagates: the fork belongs to the same owner.
            metadata: source.metadata.clone(),
            forked_from: Some(req.session_id.clone()),
            // The draft is the SOURCE session's unsent input, not history.
            draft: None,
            created_at: now,
            updated_at: now,
            message_count,
        };
        self.store.put_meta(&meta).await?;

        let event = created_event(&meta);
        Ok((
            ForkResponse {
                session_id: new_session_id,
                meta,
            },
            vec![event],
        ))
    }

    pub async fn set_active_leaf(
        &self,
        req: SetActiveLeafRequest,
    ) -> ServiceResult<SetActiveLeafResponse> {
        let _guard = self.lock_session(&req.session_id).await;
        self.meta_or_not_found(&req.session_id).await?;

        if self
            .store
            .get_entry(&req.session_id, &req.entry_id)
            .await?
            .is_none()
        {
            return Err(SessionError::EntryNotFound(format!(
                "entry {} does not exist in session {}",
                req.entry_id, req.session_id
            )));
        }

        self.store
            .set_active_leaf(&req.session_id, &req.entry_id)
            .await?;
        Ok((
            SetActiveLeafResponse {
                active_leaf: req.entry_id,
            },
            vec![],
        ))
    }
}

/// The active path: walk parent pointers from `leaf` to the root, then
/// reverse (oldest first). The parent chain **is** the order.
fn active_path<'a>(
    by_id: &HashMap<&str, &'a SessionEntry>,
    leaf: &str,
) -> Result<Vec<&'a SessionEntry>, SessionError> {
    let mut path = Vec::new();
    let mut visited: HashSet<&str> = HashSet::new();
    let mut cursor = Some(leaf.to_string());

    while let Some(id) = cursor {
        let entry = by_id
            .get(id.as_str())
            .ok_or_else(|| SessionError::Storage(format!("path references missing entry {id}")))?;
        if !visited.insert(entry.id()) {
            return Err(SessionError::Storage(format!(
                "parent chain contains a cycle at entry {id}"
            )));
        }
        path.push(*entry);
        cursor = entry.parent_id().map(str::to_string);
    }

    path.reverse();
    Ok(path)
}

/// The `session::created` event for a session's metadata. Shared with the
/// post-reload [`resync`](crate::resync) replay so a swap re-announces sessions
/// through the exact event shape a real create fires.
pub(crate) fn created_event(meta: &SessionMeta) -> EmittableEvent {
    EmittableEvent {
        event: SessionEvent::Created(SessionCreatedEvent {
            session_id: meta.session_id.clone(),
            title: meta.title.clone(),
            description: meta.description.clone(),
            status: meta.status,
            forked_from: meta.forked_from.clone(),
            created_at: meta.created_at,
        }),
        session_metadata: meta.metadata.clone(),
    }
}

// ---------------------------------------------------------------------------
// Cursors & ordering
// ---------------------------------------------------------------------------

/// Opaque cursor for `session::list`: the last item's sort key + id,
/// tagged with the order it was issued for.
#[derive(Debug, Serialize, Deserialize)]
struct SessionsCursor {
    o: String,
    k: i64,
    id: String,
}

/// Opaque cursor for `session::messages`: the last returned entry id.
#[derive(Debug, Serialize, Deserialize)]
struct MessagesCursor {
    id: String,
}

fn encode_cursor<T: Serialize>(cursor: &T) -> String {
    BASE64.encode(serde_json::to_vec(cursor).expect("cursors always serialize"))
}

fn decode_cursor<T: DeserializeOwned>(raw: &str) -> Result<T, SessionError> {
    let bytes = BASE64
        .decode(raw)
        .map_err(|e| SessionError::InvalidCursor(format!("cursor is not valid base64: {e}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| SessionError::InvalidCursor(format!("cursor payload is malformed: {e}")))
}

fn order_tag(order: ListOrder) -> &'static str {
    match order {
        ListOrder::CreatedAsc => "created_asc",
        ListOrder::CreatedDesc => "created_desc",
        ListOrder::UpdatedDesc => "updated_desc",
    }
}

fn sort_key(meta: &SessionMeta, order: ListOrder) -> (i64, &str) {
    match order {
        ListOrder::CreatedAsc | ListOrder::CreatedDesc => (meta.created_at, &meta.session_id),
        ListOrder::UpdatedDesc => (meta.updated_at, &meta.session_id),
    }
}

/// Compare two (key, id) pairs in the requested output order. The id
/// tie-break makes pagination deterministic when timestamps collide.
fn cmp_in_order<A: AsRef<str>, B: AsRef<str>>(
    a: (i64, A),
    b: (i64, B),
    order: ListOrder,
) -> std::cmp::Ordering {
    let natural = a.0.cmp(&b.0).then_with(|| a.1.as_ref().cmp(b.1.as_ref()));
    match order {
        ListOrder::CreatedAsc => natural,
        ListOrder::CreatedDesc | ListOrder::UpdatedDesc => natural.reverse(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_roundtrip() {
        let cursor = SessionsCursor {
            o: "updated_desc".into(),
            k: 42,
            id: "s_1".into(),
        };
        let encoded = encode_cursor(&cursor);
        let decoded: SessionsCursor = decode_cursor(&encoded).unwrap();
        assert_eq!(decoded.o, "updated_desc");
        assert_eq!(decoded.k, 42);
        assert_eq!(decoded.id, "s_1");
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(matches!(
            decode_cursor::<SessionsCursor>("!!!not-base64!!!"),
            Err(SessionError::InvalidCursor(_))
        ));
        let not_json = BASE64.encode(b"not json");
        assert!(matches!(
            decode_cursor::<SessionsCursor>(&not_json),
            Err(SessionError::InvalidCursor(_))
        ));
    }

    #[test]
    fn ordering_ties_break_by_id() {
        use std::cmp::Ordering;
        assert_eq!(
            cmp_in_order((5, "a"), (5, "b"), ListOrder::CreatedAsc),
            Ordering::Less
        );
        assert_eq!(
            cmp_in_order((5, "a"), (5, "b"), ListOrder::CreatedDesc),
            Ordering::Greater
        );
        assert_eq!(
            cmp_in_order((4, "z"), (5, "a"), ListOrder::UpdatedDesc),
            Ordering::Greater
        );
    }
}
