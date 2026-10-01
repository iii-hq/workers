//! A bounded display archive, independent of native credentials and protocol secrets.
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use diesel::{
    connection::SimpleConnection,
    prelude::*,
    sql_types::{BigInt, Integer, Nullable, Text},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use whatsapp_rust::types::{durability_hook::InboundDurabilityHook, events::LazyHistorySync};
use whatsapp_rust::{
    Client, SendResult,
    prelude::{Event as NativeEvent, InboundMessage},
};

use crate::{registry::Registry, sessions::private_file, validate};

const MAX_TEXT_BYTES: usize = 16_384;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MessageView {
    pub message_id: String,
    pub chat_id: String,
    pub sender_id: String,
    pub sender_name: Option<String>,
    pub from_me: bool,
    pub timestamp_ms: i64,
    pub kind: String,
    pub text: String,
    pub source: String,
    pub status: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Coverage {
    pub capture_started_at_ms: i64,
    pub last_history_sync_at_ms: Option<i64>,
    pub last_history_sync_progress: Option<u32>,
    pub message_limit: usize,
    pub retention_truncated: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MessagePage {
    /// Newest first. A cursor loads older locally retained messages.
    pub messages: Vec<MessageView>,
    pub next_cursor: Option<String>,
    pub coverage: Coverage,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ChatView {
    pub chat_id: String,
    pub title: String,
    pub is_group: bool,
    pub last_message: Option<MessageView>,
    pub message_count: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ChatPage {
    pub chats: Vec<ChatView>,
    pub next_cursor: Option<String>,
    pub coverage: Coverage,
}

struct Inner {
    connection: Option<SqliteConnection>,
    generation: u64,
    limit: usize,
}

pub struct Inbox {
    inner: Mutex<Inner>,
}

/// A capture belongs to one session/account generation. Late asynchronous work
/// cannot restore rows after logout, deletion or a different account pairing.
#[derive(Clone)]
pub struct InboxCapture {
    inbox: Arc<Inbox>,
    generation: u64,
}

#[derive(QueryableByName)]
struct MessageRow {
    #[diesel(sql_type = BigInt)]
    row_id: i64,
    #[diesel(sql_type = Text)]
    message_id: String,
    #[diesel(sql_type = Text)]
    chat_id: String,
    #[diesel(sql_type = Text)]
    sender_id: String,
    #[diesel(sql_type = Nullable<Text>)]
    sender_name: Option<String>,
    #[diesel(sql_type = Integer)]
    from_me: i32,
    #[diesel(sql_type = BigInt)]
    timestamp_ms: i64,
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = Text)]
    text: String,
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = Nullable<Text>)]
    status: Option<String>,
}

impl From<MessageRow> for MessageView {
    fn from(row: MessageRow) -> Self {
        Self {
            message_id: row.message_id,
            chat_id: row.chat_id,
            sender_id: row.sender_id,
            sender_name: row.sender_name,
            from_me: row.from_me != 0,
            timestamp_ms: row.timestamp_ms,
            kind: row.kind,
            text: row.text,
            source: row.source,
            status: row.status,
        }
    }
}

#[derive(QueryableByName)]
struct ChatRow {
    #[diesel(sql_type = Text)]
    chat_id: String,
    #[diesel(sql_type = Text)]
    title: String,
    #[diesel(sql_type = Integer)]
    is_group: i32,
    #[diesel(sql_type = BigInt)]
    timestamp_ms: i64,
    #[diesel(sql_type = BigInt)]
    message_count: i64,
}

#[derive(QueryableByName)]
struct MetaRow {
    #[diesel(sql_type = BigInt)]
    capture_started_at_ms: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    last_history_sync_at_ms: Option<i64>,
    #[diesel(sql_type = Nullable<Integer>)]
    last_history_sync_progress: Option<i32>,
    #[diesel(sql_type = Integer)]
    retention_truncated: i32,
    #[diesel(sql_type = Nullable<Text>)]
    account: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    scope: String,
    query: String,
    timestamp_ms: i64,
    key: String,
}

struct CapturedMessage {
    view: MessageView,
    expires_at_ms: Option<i64>,
    edited: bool,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

impl Inbox {
    /// Does not open credentials, create a native client, or connect to WhatsApp.
    pub fn open(directory: &Path, limit: usize) -> Result<Arc<Self>> {
        ensure!(
            (1..=100_000).contains(&limit),
            "max_inbox_messages must be 1..100000"
        );
        let path = directory.join("inbox.db");
        private_file(&path)?;
        for suffix in ["inbox.db-journal", "inbox.db-wal", "inbox.db-shm"] {
            let companion = directory.join(suffix);
            if fs::symlink_metadata(&companion).is_ok() {
                private_file(&companion)?;
            }
        }
        let mut connection =
            SqliteConnection::establish(path.to_str().context("inbox path must be UTF-8")?)?;
        // One connection owns this separate file. DELETE journal leaves no stale
        // WAL body copies; secure_delete scrubs expired/deleted text from pages.
        connection.batch_execute("PRAGMA busy_timeout=5000; PRAGMA secure_delete=ON; PRAGMA journal_mode=DELETE;
            CREATE TABLE IF NOT EXISTS inbox_meta (
                id INTEGER PRIMARY KEY CHECK(id=1), capture_started_at_ms BIGINT NOT NULL,
                last_history_sync_at_ms BIGINT, last_history_sync_progress INTEGER,
                retention_truncated INTEGER NOT NULL DEFAULT 0, account TEXT);
            CREATE TABLE IF NOT EXISTS inbox_chats (
                chat_id TEXT PRIMARY KEY, title TEXT NOT NULL, is_group INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS inbox_messages (
                row_id INTEGER PRIMARY KEY AUTOINCREMENT, chat_id TEXT NOT NULL,
                sender_key TEXT NOT NULL, message_id TEXT NOT NULL, sender_id TEXT NOT NULL,
                sender_name TEXT, from_me INTEGER NOT NULL, timestamp_ms BIGINT NOT NULL,
                kind TEXT NOT NULL, text TEXT NOT NULL, source TEXT NOT NULL, status TEXT,
                expires_at_ms BIGINT, edited INTEGER NOT NULL DEFAULT 0, UNIQUE(chat_id,sender_key,message_id));
            CREATE INDEX IF NOT EXISTS inbox_chat_order ON inbox_messages(chat_id,timestamp_ms DESC,row_id DESC);
            CREATE INDEX IF NOT EXISTS inbox_retention_order ON inbox_messages(timestamp_ms DESC,row_id DESC);
            CREATE INDEX IF NOT EXISTS inbox_expiration ON inbox_messages(expires_at_ms) WHERE expires_at_ms IS NOT NULL;")?;
        diesel::sql_query("INSERT OR IGNORE INTO inbox_meta(id,capture_started_at_ms) VALUES(1,?)")
            .bind::<BigInt, _>(now_ms())
            .execute(&mut connection)?;
        trim(&mut connection, limit)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(Inner {
                connection: Some(connection),
                generation: 0,
                limit,
            }),
        }))
    }

    pub fn capture(self: &Arc<Self>) -> InboxCapture {
        InboxCapture {
            inbox: self.clone(),
            generation: self.inner.lock().unwrap().generation,
        }
    }

    pub fn invalidate(&self) {
        self.inner.lock().unwrap().generation += 1;
    }

    pub fn set_limit(&self, limit: usize) -> Result<()> {
        ensure!(
            (1..=100_000).contains(&limit),
            "max_inbox_messages must be 1..100000"
        );
        let mut inner = self.inner.lock().unwrap();
        trim(inner.connection.as_mut().context("inbox is closed")?, limit)?;
        inner.limit = limit;
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        inner.generation += 1;
        clear(inner.connection.as_mut().context("inbox is closed")?)
    }

    pub fn set_account(&self, account: &str) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let old = metadata(inner.connection.as_mut().context("inbox is closed")?)?.account;
        if old.as_deref().is_some_and(|old| old != account) {
            inner.generation += 1;
            clear(inner.connection.as_mut().unwrap())?;
        }
        diesel::sql_query("UPDATE inbox_meta SET account=? WHERE id=1")
            .bind::<Text, _>(account)
            .execute(inner.connection.as_mut().unwrap())?;
        Ok(())
    }

    /// Waits for in-flight writes and prevents all later reads/writes. The caller
    /// can then remove the session directory without a writer recreating it.
    pub fn close(&self) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        inner.generation += 1;
        inner.connection.take();
        Ok(())
    }

    /// Offline migration of the previous process's private event snapshot. Only
    /// whitelisted display fields are persisted; this is intentionally not an RPC.
    pub fn import_event_page(&self, page: &Value) -> Result<usize> {
        let events = page
            .as_array()
            .or_else(|| page.get("events").and_then(Value::as_array))
            .context("expected an event array or event page")?;
        ensure!(
            events.len() <= 100_000,
            "event snapshot exceeds import limit"
        );
        let messages: Vec<_> = events
            .iter()
            .filter(|e| e.get("kind").and_then(Value::as_str) == Some("message.received"))
            .filter_map(|event| from_event_data(&event["data"], "live"))
            .collect();
        let count = messages.len();
        let mut inner = self.inner.lock().unwrap();
        let limit = inner.limit;
        commit(
            inner.connection.as_mut().context("inbox is closed")?,
            &messages,
            &[],
            limit,
        )?;
        Ok(count)
    }

    pub fn list_messages(
        &self,
        chat_id: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<MessagePage> {
        let chat_id = canonical_chat(chat_id).context("invalid inbox chat_id")?;
        check_limit(limit)?;
        let cursor = decode_cursor(cursor, &chat_id, "")?;
        let mut inner = self.inner.lock().unwrap();
        let retained = inner.limit;
        let conn = inner.connection.as_mut().context("inbox is closed")?;
        expire(conn)?;
        let before_time = cursor.as_ref().map_or(i64::MAX, |v| v.timestamp_ms);
        let before_id = cursor
            .as_ref()
            .map(|v| v.key.parse::<i64>())
            .transpose()?
            .unwrap_or(i64::MAX);
        let mut rows = diesel::sql_query("SELECT * FROM inbox_messages WHERE chat_id=? AND (timestamp_ms<? OR (timestamp_ms=? AND row_id<?)) ORDER BY timestamp_ms DESC,row_id DESC LIMIT ?")
            .bind::<Text,_>(&chat_id).bind::<BigInt,_>(before_time).bind::<BigInt,_>(before_time).bind::<BigInt,_>(before_id).bind::<BigInt,_>((limit+1) as i64).load::<MessageRow>(conn)?;
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next_cursor = if more {
            rows.last()
                .map(|last| encode_cursor(&chat_id, "", last.timestamp_ms, last.row_id.to_string()))
                .transpose()?
        } else {
            None
        };
        Ok(MessagePage {
            messages: rows.into_iter().map(Into::into).collect(),
            next_cursor,
            coverage: coverage(conn, retained)?,
        })
    }

    pub fn list_chats(
        &self,
        query: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ChatPage> {
        check_limit(limit)?;
        let query = query.unwrap_or("").trim();
        ensure!(query.len() <= 256, "query must be at most 256 bytes");
        let cursor = decode_cursor(cursor, "chats", query)?;
        let mut inner = self.inner.lock().unwrap();
        let retained = inner.limit;
        let conn = inner.connection.as_mut().context("inbox is closed")?;
        expire(conn)?;
        let before_time = cursor.as_ref().map_or(i64::MAX, |v| v.timestamp_ms);
        let before_key = cursor.as_ref().map_or("\u{10ffff}", |v| v.key.as_str());
        let mut rows = diesel::sql_query("SELECT c.chat_id,c.title,c.is_group,COALESCE(MAX(m.timestamp_ms),0) AS timestamp_ms,COUNT(m.row_id) AS message_count
            FROM inbox_chats c LEFT JOIN inbox_messages m ON m.chat_id=c.chat_id
            WHERE (?='' OR instr(lower(c.title),lower(?))>0 OR instr(c.chat_id,?)>0 OR EXISTS(SELECT 1 FROM inbox_messages search WHERE search.chat_id=c.chat_id AND instr(lower(search.text),lower(?))>0))
            GROUP BY c.chat_id HAVING COALESCE(MAX(m.timestamp_ms),0)<? OR (COALESCE(MAX(m.timestamp_ms),0)=? AND c.chat_id<?)
            ORDER BY timestamp_ms DESC,c.chat_id DESC LIMIT ?")
            .bind::<Text,_>(query).bind::<Text,_>(query).bind::<Text,_>(query).bind::<Text,_>(query)
            .bind::<BigInt,_>(before_time).bind::<BigInt,_>(before_time).bind::<Text,_>(before_key).bind::<BigInt,_>((limit+1) as i64).load::<ChatRow>(conn)?;
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next_cursor = if more {
            rows.last()
                .map(|last| encode_cursor("chats", query, last.timestamp_ms, last.chat_id.clone()))
                .transpose()?
        } else {
            None
        };
        let mut chats = Vec::with_capacity(rows.len());
        for row in rows {
            let last_message = diesel::sql_query("SELECT * FROM inbox_messages WHERE chat_id=? ORDER BY timestamp_ms DESC,row_id DESC LIMIT 1").bind::<Text,_>(&row.chat_id).get_result::<MessageRow>(conn).optional()?.map(Into::into);
            chats.push(ChatView {
                chat_id: row.chat_id,
                title: row.title,
                is_group: row.is_group != 0,
                last_message,
                message_count: row.message_count as usize,
            });
        }
        Ok(ChatPage {
            chats,
            next_cursor,
            coverage: coverage(conn, retained)?,
        })
    }

    fn oldest(&self, chat_id: &str) -> Result<MessageView> {
        let chat = canonical_chat(chat_id).context("invalid inbox chat_id")?;
        let mut inner = self.inner.lock().unwrap();
        let conn = inner.connection.as_mut().context("inbox is closed")?;
        diesel::sql_query("SELECT * FROM inbox_messages WHERE chat_id=? AND timestamp_ms>0 ORDER BY timestamp_ms,row_id LIMIT 1").bind::<Text,_>(chat).get_result::<MessageRow>(conn).optional()?.map(Into::into).context("history requires a known message in this chat; wait for a message or received sync")
    }
}

fn check_limit(limit: usize) -> Result<()> {
    ensure!((1..=200).contains(&limit), "limit must be 1..200");
    Ok(())
}
fn encode_cursor(scope: &str, query: &str, timestamp_ms: i64, key: String) -> Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&Cursor {
        scope: scope.into(),
        query: query.into(),
        timestamp_ms,
        key,
    })?))
}
fn decode_cursor(cursor: Option<&str>, scope: &str, query: &str) -> Result<Option<Cursor>> {
    cursor
        .map(|value| {
            ensure!(value.len() <= 2048, "invalid cursor");
            let cursor: Cursor =
                serde_json::from_slice(&URL_SAFE_NO_PAD.decode(value).context("invalid cursor")?)
                    .context("invalid cursor")?;
            ensure!(
                cursor.scope == scope && cursor.query == query,
                "cursor belongs to another chat or search"
            );
            Ok(cursor)
        })
        .transpose()
}
fn metadata(conn: &mut SqliteConnection) -> Result<MetaRow> {
    Ok(diesel::sql_query("SELECT * FROM inbox_meta WHERE id=1").get_result(conn)?)
}
fn coverage(conn: &mut SqliteConnection, limit: usize) -> Result<Coverage> {
    let meta = metadata(conn)?;
    Ok(Coverage {
        capture_started_at_ms: meta.capture_started_at_ms,
        last_history_sync_at_ms: meta.last_history_sync_at_ms,
        last_history_sync_progress: meta.last_history_sync_progress.map(|v| v as u32),
        message_limit: limit,
        retention_truncated: meta.retention_truncated != 0,
    })
}
fn expire(conn: &mut SqliteConnection) -> Result<()> {
    diesel::sql_query("UPDATE inbox_messages SET text='',kind='expired' WHERE expires_at_ms<=? AND kind!='expired'").bind::<BigInt,_>(now_ms()).execute(conn)?;
    Ok(())
}
fn trim(conn: &mut SqliteConnection, limit: usize) -> Result<()> {
    expire(conn)?;
    let deleted=diesel::sql_query("DELETE FROM inbox_messages WHERE row_id IN (SELECT row_id FROM inbox_messages ORDER BY timestamp_ms DESC,row_id DESC LIMIT -1 OFFSET ?)").bind::<BigInt,_>(limit as i64).execute(conn)?;
    if deleted > 0 {
        diesel::sql_query("UPDATE inbox_meta SET retention_truncated=1 WHERE id=1")
            .execute(conn)?;
    }
    // Bound metadata as well, including history chunks with many empty chats.
    diesel::sql_query("DELETE FROM inbox_chats WHERE chat_id NOT IN (SELECT chat_id FROM inbox_messages) AND chat_id IN (SELECT chat_id FROM inbox_chats ORDER BY chat_id LIMIT -1 OFFSET ?)").bind::<BigInt,_>(limit as i64).execute(conn)?;
    Ok(())
}
fn clear(conn: &mut SqliteConnection) -> Result<()> {
    conn.transaction::<_,anyhow::Error,_>(|conn|{conn.batch_execute("DELETE FROM inbox_messages; DELETE FROM inbox_chats;")?;diesel::sql_query("UPDATE inbox_meta SET capture_started_at_ms=?,last_history_sync_at_ms=NULL,last_history_sync_progress=NULL,retention_truncated=0,account=NULL WHERE id=1").bind::<BigInt,_>(now_ms()).execute(conn)?;Ok(())})
}

fn commit(
    conn: &mut SqliteConnection,
    messages: &[CapturedMessage],
    chats: &[(String, String)],
    limit: usize,
) -> Result<()> {
    conn.transaction::<_,anyhow::Error,_>(|conn| {
        for (id,title) in chats {
            diesel::sql_query("INSERT INTO inbox_chats(chat_id,title,is_group) VALUES(?,?,?) ON CONFLICT(chat_id) DO UPDATE SET title=CASE WHEN excluded.title!='' THEN excluded.title ELSE inbox_chats.title END")
                .bind::<Text,_>(id).bind::<Text,_>(clipped(title,512)).bind::<Integer,_>(i32::from(id.ends_with("@g.us"))).execute(conn)?;
        }
        for message in messages {
            let m=&message.view;
            let title=if !m.from_me&&!m.chat_id.ends_with("@g.us") {m.sender_name.as_deref().unwrap_or(&m.chat_id)} else {&m.chat_id};
            diesel::sql_query("INSERT INTO inbox_chats(chat_id,title,is_group) VALUES(?,?,?) ON CONFLICT(chat_id) DO UPDATE SET title=CASE WHEN inbox_chats.title=inbox_chats.chat_id THEN excluded.title ELSE inbox_chats.title END")
                .bind::<Text,_>(&m.chat_id).bind::<Text,_>(title).bind::<Integer,_>(i32::from(m.chat_id.ends_with("@g.us"))).execute(conn)?;
            diesel::sql_query("INSERT INTO inbox_messages(chat_id,sender_key,message_id,sender_id,sender_name,from_me,timestamp_ms,kind,text,source,status,expires_at_ms,edited) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)
                ON CONFLICT(chat_id,sender_key,message_id) DO UPDATE SET
                sender_name=COALESCE(excluded.sender_name,inbox_messages.sender_name),
                kind=CASE WHEN inbox_messages.kind IN ('view_once','expired','deleted') THEN inbox_messages.kind WHEN inbox_messages.edited=1 AND excluded.edited=0 AND excluded.kind!='deleted' THEN inbox_messages.kind ELSE excluded.kind END,
                text=CASE WHEN inbox_messages.kind IN ('view_once','expired','deleted') OR excluded.kind='deleted' THEN '' WHEN inbox_messages.edited=1 AND excluded.edited=0 THEN inbox_messages.text ELSE excluded.text END,
                expires_at_ms=COALESCE(inbox_messages.expires_at_ms,excluded.expires_at_ms),
                edited=MAX(inbox_messages.edited,excluded.edited)")
                .bind::<Text,_>(&m.chat_id).bind::<Text,_>(if m.from_me{"me"}else{&m.sender_id}).bind::<Text,_>(&m.message_id).bind::<Text,_>(&m.sender_id)
                .bind::<Nullable<Text>,_>(m.sender_name.as_deref()).bind::<Integer,_>(i32::from(m.from_me)).bind::<BigInt,_>(m.timestamp_ms).bind::<Text,_>(&m.kind).bind::<Text,_>(&m.text).bind::<Text,_>(&m.source).bind::<Nullable<Text>,_>(m.status.as_deref()).bind::<Nullable<BigInt>,_>(message.expires_at_ms).bind::<Integer,_>(i32::from(message.edited)).execute(conn)?;
        }
        trim(conn,limit)
    })
}

impl InboxCapture {
    fn with_connection<T>(
        &self,
        f: impl FnOnce(&mut SqliteConnection, usize) -> Result<T>,
    ) -> Result<Option<T>> {
        let mut inner = self.inbox.inner.lock().unwrap();
        if inner.generation != self.generation || inner.connection.is_none() {
            return Ok(None);
        }
        let limit = inner.limit;
        f(inner.connection.as_mut().unwrap(), limit).map(Some)
    }

    fn inbound(&self, batch: &[InboundMessage]) -> Result<()> {
        let messages=batch.iter().filter_map(|message|from_event_data(&json!({
            "info":{"id":message.info.id.to_string(),"timestamp_ms":message.info.timestamp.timestamp_millis(),"push_name":message.info.push_name,
                "source":{"chat":message.info.source.chat.to_string(),"sender":message.info.source.sender.to_string(),"is_from_me":message.info.source.is_from_me}},
            "message":message.message,"ephemeral_expiration":message.ephemeral_expiration
        }),"live")).collect::<Vec<_>>();
        ensure!(
            self.with_connection(|conn, limit| commit(conn, &messages, &[], limit))?
                .is_some(),
            "inbox capture was invalidated"
        );
        Ok(())
    }

    pub async fn record_sent(&self, result: &SendResult) -> Result<()> {
        let capture = self.clone();
        let result = result.clone();
        tokio::task::spawn_blocking(move || {
            if let Some(message) = normalize(
                &result.to.to_string(),
                "me",
                &result.message_id,
                None,
                true,
                now_ms(),
                &serde_json::to_value(result.message)?,
                None,
                None,
                "sent",
            ) {
                capture.with_connection(|conn, limit| commit(conn, &[message], &[], limit))?;
            }
            Ok(())
        })
        .await?
    }

    pub(crate) fn ingest_event(&self, event: Arc<NativeEvent>) {
        let capture = self.clone();
        tokio::task::spawn_blocking(move || {
            let result = match &*event {
                NativeEvent::Messages(batch) if !batch.hook_committed => {
                    capture.inbound(&batch.messages)
                }
                NativeEvent::HistorySync(sync) => capture.history(sync),
                NativeEvent::Receipt(receipt) => capture.receipt(receipt),
                _ => Ok(()),
            };
            if result.is_err() {
                tracing::warn!("could not update WhatsApp inbox archive");
            }
        });
    }

    fn history(&self, sync: &LazyHistorySync) -> Result<()> {
        let mut stream = sync.stream();
        while let Some(conversation) = stream.next_conversation()? {
            let Some(chat) = canonical_chat(&conversation.id) else {
                continue;
            };
            let title = conversation
                .name
                .as_deref()
                .or(conversation.display_name.as_deref())
                .unwrap_or(&chat)
                .to_owned();
            let mut messages = Vec::with_capacity(256);
            for entry in &conversation.messages {
                let Some(web) = entry.message.as_option() else {
                    continue;
                };
                let Some(body) = web.message.as_option() else {
                    continue;
                };
                let Some(key) = web.key.as_option() else {
                    continue;
                };
                let Some(id) = key.id.as_deref() else {
                    continue;
                };
                let from_me = key.from_me.unwrap_or(false);
                let sender = if from_me {
                    "me"
                } else {
                    key.participant
                        .as_deref()
                        .or(web.participant.as_deref())
                        .unwrap_or(&chat)
                };
                let timestamp = web
                    .message_timestamp
                    .and_then(|t| i64::try_from(t).ok())
                    .and_then(|t| t.checked_mul(1000))
                    .unwrap_or(0);
                let expiration = web.ephemeral_duration;
                let start = web
                    .ephemeral_start_timestamp
                    .and_then(|t| i64::try_from(t).ok())
                    .and_then(|t| t.checked_mul(1000));
                if let Some(mut message) = normalize(
                    &chat,
                    sender,
                    id,
                    web.push_name.as_deref(),
                    from_me,
                    timestamp,
                    &serde_json::to_value(body)?,
                    expiration,
                    start,
                    "history",
                ) {
                    let absolute_expiry = web
                        .ephemeral_expiration_timestamp
                        .and_then(|t| i64::try_from(t).ok())
                        .and_then(|t| t.checked_mul(1000));
                    if let Some(expiry) = absolute_expiry {
                        message.expires_at_ms = Some(
                            message
                                .expires_at_ms
                                .map_or(expiry, |existing| existing.min(expiry)),
                        );
                        if expiry <= now_ms() {
                            message.view.kind = "expired".into();
                            message.view.text.clear();
                        }
                    } else if web.ephemeral_start_timestamp.is_some()
                        && expiration.filter(|v| *v > 0).is_none()
                    {
                        message.view.kind = "ephemeral".into();
                        message.view.text.clear();
                    }
                    messages.push(message);
                }
                if messages.len() == 256 {
                    if self
                        .with_connection(|conn, limit| {
                            commit(conn, &messages, &[(chat.clone(), title.clone())], limit)
                        })?
                        .is_none()
                    {
                        return Ok(());
                    }
                    messages.clear();
                }
            }
            if self
                .with_connection(|conn, limit| commit(conn, &messages, &[(chat, title)], limit))?
                .is_none()
            {
                return Ok(());
            }
        }
        self.with_connection(|conn,_|{
            diesel::sql_query("UPDATE inbox_meta SET last_history_sync_at_ms=?,last_history_sync_progress=? WHERE id=1").bind::<BigInt,_>(now_ms()).bind::<Nullable<Integer>,_>(sync.progress().map(|p|p.min(100)as i32)).execute(conn)?;
            Ok(())
        })?;
        Ok(())
    }

    fn receipt(&self, receipt: &whatsapp_rust::types::events::Receipt) -> Result<()> {
        use whatsapp_rust::types::presence::ReceiptType;
        // Group receipts describe individual members, not the whole group's
        // delivery state. Leave them in the diagnostic event stream.
        if receipt.source.is_group {
            return Ok(());
        }
        let (status, rank) = match receipt.r#type {
            ReceiptType::Delivered => ("delivered", 2),
            ReceiptType::Read => ("read", 3),
            ReceiptType::Played => ("played", 4),
            _ => return Ok(()),
        };
        let Some(chat) = canonical_chat(&receipt.source.chat.to_string()) else {
            return Ok(());
        };
        self.with_connection(|conn,_|{
            for id in &receipt.message_ids {
                diesel::sql_query("UPDATE inbox_messages SET status=? WHERE chat_id=? AND message_id=? AND from_me=1 AND CASE status WHEN 'played' THEN 4 WHEN 'read' THEN 3 WHEN 'delivered' THEN 2 ELSE 1 END<?")
                    .bind::<Text,_>(status).bind::<Text,_>(&chat).bind::<Text,_>(id.as_str()).bind::<Integer,_>(rank).execute(conn)?;
            }Ok(())
        })?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl InboundDurabilityHook for InboxCapture {
    async fn on_messages(&self, _client: Arc<Client>, batch: &[InboundMessage]) -> Result<()> {
        let capture = self.clone();
        let batch = batch.to_vec();
        tokio::task::spawn_blocking(move || capture.inbound(&batch)).await?
    }
}

fn canonical_chat(value: &str) -> Option<String> {
    use whatsapp_rust::Server;
    let jid = validate::jid(value).ok()?;
    matches!(jid.server, Server::Pn | Server::Lid | Server::Group)
        .then(|| jid.to_non_ad().to_string())
}
fn clipped(value: &str, max: usize) -> String {
    let mut end = value.len().min(max);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}
fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    value.get(name).filter(|v| !v.is_null())
}
fn from_event_data(data: &Value, source: &str) -> Option<CapturedMessage> {
    let info = data.get("info")?;
    let src = info.get("source")?;
    normalize(
        src.get("chat")?.as_str()?,
        src.get("sender")?.as_str()?,
        info.get("id")?.as_str()?,
        info.get("push_name").and_then(Value::as_str),
        src.get("is_from_me")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        info.get("timestamp_ms")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        data.get("message")?,
        data.get("ephemeral_expiration")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
        None,
        source,
    )
}

#[allow(clippy::too_many_arguments)]
fn normalize(
    chat: &str,
    sender: &str,
    id: &str,
    name: Option<&str>,
    from_me: bool,
    timestamp_ms: i64,
    body: &Value,
    expiration: Option<u32>,
    ephemeral_start: Option<i64>,
    source: &str,
) -> Option<CapturedMessage> {
    let chat = canonical_chat(chat)?;
    if id.is_empty() || id.len() > 256 {
        return None;
    }
    let sender = if from_me {
        "me".to_owned()
    } else {
        canonical_chat(sender).unwrap_or_else(|| clipped(sender, 256))
    };
    let mut current = body;
    let mut ephemeral = false;
    let mut once = false;
    for _ in 0..12 {
        if [
            "view_once_message",
            "view_once_message_v2",
            "view_once_message_v2_extension",
        ]
        .iter()
        .any(|k| field(current, k).is_some())
        {
            once = true;
            break;
        }
        let next = if let Some(wrapper) = field(current, "ephemeral_message") {
            ephemeral = true;
            wrapper.get("message")
        } else if let Some(wrapper) = field(current, "device_sent_message") {
            wrapper.get("message")
        } else {
            None
        };
        match next {
            Some(next) => current = next,
            None => break,
        }
    }
    if let Some(protocol) = field(current, "protocol_message") {
        let kind = protocol.get("type")?;
        let revoke = kind
            .as_str()
            .is_some_and(|v| v.eq_ignore_ascii_case("revoke"))
            || kind.as_i64() == Some(0);
        let edit = kind
            .as_str()
            .is_some_and(|v| v.eq_ignore_ascii_case("message_edit"))
            || kind.as_i64() == Some(14);
        if !revoke && !edit {
            return None;
        }
        let key = protocol.get("key")?;
        let target_id = key.get("id")?.as_str()?;
        if target_id.is_empty() || target_id.len() > 256 {
            return None;
        }
        if let Some(target_chat) = key.get("remote_jid").and_then(Value::as_str)
            && canonical_chat(target_chat).as_deref() != Some(chat.as_str())
        {
            return None;
        }
        let target_from_me = key
            .get("from_me")
            .and_then(Value::as_bool)
            .unwrap_or(from_me);
        let target_sender = if target_from_me {
            "me"
        } else {
            key.get("participant")
                .and_then(Value::as_str)
                .unwrap_or(&sender)
        };
        let mut target = if revoke {
            normalize(
                &chat,
                target_sender,
                target_id,
                name,
                target_from_me,
                timestamp_ms,
                &json!({"conversation":""}),
                None,
                None,
                source,
            )?
        } else {
            let body = protocol.get("edited_message")?;
            if field(body, "protocol_message").is_some() {
                return None;
            }
            normalize(
                &chat,
                target_sender,
                target_id,
                name,
                target_from_me,
                timestamp_ms,
                body,
                expiration,
                ephemeral_start,
                source,
            )?
        };
        if revoke {
            target.view.kind = "deleted".to_owned();
            target.view.text.clear();
        } else {
            target.edited = true;
        }
        return Some(target);
    }
    let context_expiration = current
        .as_object()
        .into_iter()
        .flat_map(|obj| obj.values())
        .filter_map(|v| v.get("context_info"))
        .find_map(|c| c.get("expiration").and_then(Value::as_u64))
        .and_then(|n| u32::try_from(n).ok());
    once |= ["image_message", "video_message", "audio_message"]
        .iter()
        .any(|key| {
            current
                .get(key)
                .and_then(|media| media.get("view_once"))
                .and_then(Value::as_bool)
                == Some(true)
        });
    let duration = expiration
        .filter(|v| *v > 0)
        .or(context_expiration.filter(|v| *v > 0));
    let expires_at_ms = duration.and_then(|seconds| {
        ephemeral_start
            .unwrap_or(timestamp_ms)
            .checked_add(i64::from(seconds) * 1000)
    });
    let expired = expires_at_ms.is_some_and(|expires| expires <= now_ms());
    let (kind, text) = if once {
        ("view_once", String::new())
    } else if expired {
        ("expired", String::new())
    } else if ephemeral && duration.is_none() {
        ("ephemeral", String::new())
    } else if let Some(text) = current.get("conversation").and_then(Value::as_str) {
        ("text", clipped(text, MAX_TEXT_BYTES))
    } else if let Some(text) = current
        .get("extended_text_message")
        .and_then(|m| m.get("text"))
        .and_then(Value::as_str)
    {
        ("text", clipped(text, MAX_TEXT_BYTES))
    } else {
        let mut found = None;
        for (key, kind, caption) in [
            ("image_message", "image", "caption"),
            ("video_message", "video", "caption"),
            ("audio_message", "audio", ""),
            ("document_message", "document", "file_name"),
            ("sticker_message", "sticker", ""),
            ("contact_message", "contact", "display_name"),
            ("contacts_array_message", "contact", "display_name"),
            ("location_message", "location", "name"),
            ("live_location_message", "location", "caption"),
            ("poll_creation_message", "poll", "name"),
            ("poll_creation_message_v2", "poll", "name"),
            ("poll_creation_message_v3", "poll", "name"),
            ("reaction_message", "reaction", "text"),
        ] {
            if let Some(message) = field(current, key) {
                found = Some((
                    kind,
                    clipped(
                        message.get(caption).and_then(Value::as_str).unwrap_or(""),
                        MAX_TEXT_BYTES,
                    ),
                ));
                break;
            }
        }
        // Protocol, cryptographic and unsupported control envelopes are not
        // conversations and must never enter the display archive.
        found?
    };
    Some(CapturedMessage {
        view: MessageView {
            message_id: id.to_owned(),
            chat_id: chat,
            sender_id: sender,
            sender_name: name
                .filter(|v| !v.trim().is_empty())
                .map(|v| clipped(v, 512)),
            from_me,
            timestamp_ms: timestamp_ms.max(0),
            kind: kind.to_owned(),
            text,
            source: source.to_owned(),
            status: from_me.then(|| "submitted".to_owned()),
        },
        expires_at_ms,
        edited: false,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ChatsRequest {
    session_id: String,
    query: Option<String>,
    cursor: Option<String>,
    #[serde(default = "chat_limit")]
    limit: usize,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MessagesRequest {
    session_id: String,
    chat_id: String,
    cursor: Option<String>,
    #[serde(default = "message_limit")]
    limit: usize,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct HistoryRequest {
    session_id: String,
    chat_id: String,
    #[serde(default = "message_limit")]
    count: usize,
}
#[derive(Serialize, JsonSchema)]
struct HistoryRequested {
    request_id: String,
    requested_count: usize,
}
fn chat_limit() -> usize {
    100
}
fn message_limit() -> usize {
    50
}

pub fn register(registry: &mut Registry) {
    registry.add::<ChatsRequest, ChatPage, _, _>(
        "whatsapp::chats::list",
        "List locally captured conversations and received history; never marks chats read",
        |manager, req| async move {
            let inbox = manager.inbox(&req.session_id)?;
            tokio::task::spawn_blocking(move || {
                inbox.list_chats(req.query.as_deref(), req.cursor.as_deref(), req.limit)
            })
            .await?
        },
    );
    registry.add::<MessagesRequest, MessagePage, _, _>(
        "whatsapp::messages::list",
        "Read retained messages newest first, without sending read receipts",
        |manager, req| async move {
            let inbox = manager.inbox(&req.session_id)?;
            tokio::task::spawn_blocking(move || {
                inbox.list_messages(&req.chat_id, req.cursor.as_deref(), req.limit)
            })
            .await?
        },
    );
    registry.add::<HistoryRequest,HistoryRequested,_,_>("whatsapp::messages::request_history","Ask the primary phone for older messages before the oldest locally known message; response arrives asynchronously",|manager,req|async move{
        ensure!((1..=100).contains(&req.count),"count must be 1..100");
        let inbox=manager.inbox(&req.session_id)?;
        let chat=req.chat_id.clone();
        let oldest=tokio::task::spawn_blocking(move||inbox.oldest(&chat)).await??;
        let client=manager.client(&req.session_id).await?;
        let request_id=client.fetch_message_history(&validate::jid(&oldest.chat_id)?,&oldest.message_id,oldest.from_me,oldest.timestamp_ms,req.count as i32).await?;
        Ok(HistoryRequested{request_id,requested_count:req.count})
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use whatsapp_rust::prelude::{MessageField, MessageInfo, wa};

    fn inbound(id: &str, text: &str) -> InboundMessage {
        let mut info = MessageInfo {
            id: id.into(),
            ..Default::default()
        };
        info.source.chat = "123@g.us".parse().unwrap();
        info.source.sender = "10@s.whatsapp.net".parse().unwrap();
        InboundMessage::builder()
            .info(Arc::new(info))
            .message(Arc::new(wa::Message {
                conversation: Some(text.into()),
                ..Default::default()
            }))
            .build()
    }

    #[test]
    fn previous_capture_cannot_restore_data_after_account_reset_or_close() {
        let dir = TempDir::new().unwrap();
        let inbox = Inbox::open(dir.path(), 100).unwrap();
        inbox.set_account("first@s.whatsapp.net").unwrap();
        let old = inbox.capture();
        old.inbound(&[inbound("a", "old account")]).unwrap();
        inbox.set_account("second@s.whatsapp.net").unwrap();
        assert!(old.inbound(&[inbound("b", "late old account")]).is_err());
        inbox
            .capture()
            .inbound(&[inbound("c", "new account")])
            .unwrap();
        let page = inbox.list_messages("123@g.us", None, 50).unwrap();
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].text, "new account");
        let latest = inbox.capture();
        inbox.close().unwrap();
        assert!(latest.inbound(&[inbound("d", "after delete")]).is_err());
    }

    // A zlib stream containing an uncompressed DEFLATE block, sufficient for a
    // small synthetic protobuf fixture without another compression dependency.
    fn stored_zlib(raw: &[u8]) -> Vec<u8> {
        assert!(raw.len() <= u16::MAX as usize);
        let size = raw.len() as u16;
        let mut bytes = vec![0x78, 0x01, 0x01];
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&(!size).to_le_bytes());
        bytes.extend_from_slice(raw);
        let (mut a, mut b) = (1u32, 0u32);
        for value in raw {
            a = (a + u32::from(*value)) % 65521;
            b = (b + a) % 65521;
        }
        bytes.extend_from_slice(&((b << 16) | a).to_be_bytes());
        bytes
    }

    #[test]
    fn native_history_stream_archives_names_messages_and_progress_in_bounded_batches() {
        let dir = TempDir::new().unwrap();
        let inbox = Inbox::open(dir.path(), 270).unwrap();
        let history = wa::HistorySync {
            sync_type: wa::history_sync::HistorySyncType::RECENT,
            conversations: vec![wa::Conversation {
                id: "123@g.us".into(),
                name: Some("Work group".into()),
                messages: (0..300)
                    .map(|i| wa::HistorySyncMsg {
                        message: MessageField::some(wa::WebMessageInfo {
                            key: MessageField::some(wa::MessageKey {
                                remote_jid: Some("123@g.us".into()),
                                participant: Some("10@s.whatsapp.net".into()),
                                id: Some(i.to_string()),
                                ..Default::default()
                            }),
                            message: MessageField::some(wa::Message {
                                conversation: Some(format!("history {i}")),
                                ..Default::default()
                            }),
                            message_timestamp: Some(1000 + i),
                            push_name: Some("Alice".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut raw = Vec::new();
        whatsapp_rust::waproto::codec::history_sync_encode_into(&history, &mut raw);
        let sync = LazyHistorySync::new(stored_zlib(&raw).into(), raw.len(), 3, Some(0), Some(100));
        inbox.capture().history(&sync).unwrap();
        let chats = inbox.list_chats(None, None, 50).unwrap();
        assert_eq!(chats.chats[0].title, "Work group");
        assert_eq!(chats.chats[0].message_count, 270);
        assert_eq!(
            chats.chats[0].last_message.as_ref().unwrap().text,
            "history 299"
        );
        assert_eq!(chats.coverage.last_history_sync_progress, Some(100));
        assert!(chats.coverage.retention_truncated);
        assert!(chats.coverage.last_history_sync_at_ms.is_some());
        assert_eq!(inbox.oldest("123@g.us").unwrap().message_id, "30");
    }

    #[test]
    fn history_absolute_expiry_and_incomplete_ephemeral_metadata_hide_contents() {
        let dir = TempDir::new().unwrap();
        let inbox = Inbox::open(dir.path(), 100).unwrap();
        let mut entries = Vec::new();
        for (id, absolute, start) in [
            ("expired", Some(1), None),
            ("unknown", None, Some(1)),
            ("future", Some((now_ms() / 1000 + 60) as u64), None),
        ] {
            entries.push(wa::HistorySyncMsg {
                message: MessageField::some(wa::WebMessageInfo {
                    key: MessageField::some(wa::MessageKey {
                        remote_jid: Some("123@g.us".into()),
                        participant: Some("10@s.whatsapp.net".into()),
                        id: Some(id.into()),
                        ..Default::default()
                    }),
                    message: MessageField::some(wa::Message {
                        conversation: Some(format!("secret {id}")),
                        ..Default::default()
                    }),
                    message_timestamp: Some(1),
                    ephemeral_expiration_timestamp: absolute,
                    ephemeral_start_timestamp: start,
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
        let history = wa::HistorySync {
            sync_type: wa::history_sync::HistorySyncType::RECENT,
            conversations: vec![wa::Conversation {
                id: "123@g.us".into(),
                messages: entries,
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut raw = Vec::new();
        whatsapp_rust::waproto::codec::history_sync_encode_into(&history, &mut raw);
        inbox
            .capture()
            .history(&LazyHistorySync::new(
                stored_zlib(&raw).into(),
                raw.len(),
                3,
                None,
                None,
            ))
            .unwrap();
        let page = inbox.list_messages("123@g.us", None, 50).unwrap();
        assert_eq!(
            page.messages
                .iter()
                .find(|m| m.message_id == "expired")
                .unwrap()
                .kind,
            "expired"
        );
        assert_eq!(
            page.messages
                .iter()
                .find(|m| m.message_id == "unknown")
                .unwrap()
                .kind,
            "ephemeral"
        );
        let encoded = serde_json::to_string(&page).unwrap();
        assert!(!encoded.contains("secret expired"));
        assert!(!encoded.contains("secret unknown"));
        assert!(encoded.contains("secret future"));
    }
}
