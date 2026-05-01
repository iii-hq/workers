//! Session storage as a parent-id tree of typed entries.
//!
//! P0 surface: create / load / append / active_path / list / load_messages.
//! P2 surface: fork / clone_session / compact / export_html / tree.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use harness_types::{AgentContext, AgentMessage, ContentBlock};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One entry in the session tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEntry {
    Message {
        id: String,
        parent_id: Option<String>,
        message: AgentMessage,
        timestamp: i64,
    },
    CustomMessage {
        id: String,
        parent_id: Option<String>,
        custom_type: String,
        content: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<String>,
        #[serde(default)]
        details: serde_json::Value,
        timestamp: i64,
    },
    BranchSummary {
        id: String,
        parent_id: Option<String>,
        summary: String,
        from_id: String,
        timestamp: i64,
    },
    Compaction {
        id: String,
        parent_id: Option<String>,
        summary: String,
        tokens_before: u64,
        details: CompactionDetails,
        timestamp: i64,
    },
}

impl SessionEntry {
    pub fn id(&self) -> &str {
        match self {
            Self::Message { id, .. }
            | Self::CustomMessage { id, .. }
            | Self::BranchSummary { id, .. }
            | Self::Compaction { id, .. } => id,
        }
    }

    pub fn parent_id(&self) -> Option<&str> {
        match self {
            Self::Message { parent_id, .. }
            | Self::CustomMessage { parent_id, .. }
            | Self::BranchSummary { parent_id, .. }
            | Self::Compaction { parent_id, .. } => parent_id.as_deref(),
        }
    }

    /// Replace the entry's id, returning a new entry.
    fn with_id(mut self, new_id: String) -> Self {
        match &mut self {
            Self::Message { id, .. }
            | Self::CustomMessage { id, .. }
            | Self::BranchSummary { id, .. }
            | Self::Compaction { id, .. } => *id = new_id,
        }
        self
    }

    /// Replace the entry's parent_id, returning a new entry.
    fn with_parent(mut self, new_parent: Option<String>) -> Self {
        match &mut self {
            Self::Message { parent_id, .. }
            | Self::CustomMessage { parent_id, .. }
            | Self::BranchSummary { parent_id, .. }
            | Self::Compaction { parent_id, .. } => *parent_id = new_parent,
        }
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionDetails {
    #[serde(default)]
    pub read_files: Vec<String>,
    #[serde(default)]
    pub modified_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default)]
    pub branch_count: u32,
}

/// Nested tree representation of a session, rooted at the entry whose
/// `parent_id` is `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreeNode {
    pub entry: SessionEntry,
    pub children: Vec<TreeNode>,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session not found: {0}")]
    NotFound(String),
    #[error("entry not found: {0}")]
    EntryNotFound(String),
    #[error("storage error: {0}")]
    Storage(String),
}

/// Storage backend abstraction.
#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn create(&self, meta: SessionMeta) -> Result<(), SessionError>;
    async fn append(&self, session_id: &str, entry: SessionEntry) -> Result<(), SessionError>;
    async fn load_entries(&self, session_id: &str) -> Result<Vec<SessionEntry>, SessionError>;
    async fn load_meta(&self, session_id: &str) -> Result<SessionMeta, SessionError>;
    async fn list(&self) -> Result<Vec<SessionMeta>, SessionError>;
}

/// In-memory backend used by tests and replay tools.
#[derive(Debug, Clone, Default)]
pub struct InMemoryStore {
    entries: Arc<RwLock<HashMap<String, Vec<SessionEntry>>>>,
    meta: Arc<RwLock<HashMap<String, SessionMeta>>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SessionStore for InMemoryStore {
    async fn create(&self, meta: SessionMeta) -> Result<(), SessionError> {
        self.meta
            .write()
            .map_err(|e| SessionError::Storage(e.to_string()))?
            .insert(meta.session_id.clone(), meta.clone());
        self.entries
            .write()
            .map_err(|e| SessionError::Storage(e.to_string()))?
            .insert(meta.session_id, Vec::new());
        Ok(())
    }

    async fn append(&self, session_id: &str, entry: SessionEntry) -> Result<(), SessionError> {
        let mut entries = self
            .entries
            .write()
            .map_err(|e| SessionError::Storage(e.to_string()))?;
        let list = entries
            .get_mut(session_id)
            .ok_or_else(|| SessionError::NotFound(session_id.to_string()))?;
        list.push(entry);

        let mut meta = self
            .meta
            .write()
            .map_err(|e| SessionError::Storage(e.to_string()))?;
        if let Some(m) = meta.get_mut(session_id) {
            m.updated_at = chrono::Utc::now().timestamp_millis();
        }
        Ok(())
    }

    async fn load_entries(&self, session_id: &str) -> Result<Vec<SessionEntry>, SessionError> {
        self.entries
            .read()
            .map_err(|e| SessionError::Storage(e.to_string()))?
            .get(session_id)
            .cloned()
            .ok_or_else(|| SessionError::NotFound(session_id.to_string()))
    }

    async fn load_meta(&self, session_id: &str) -> Result<SessionMeta, SessionError> {
        self.meta
            .read()
            .map_err(|e| SessionError::Storage(e.to_string()))?
            .get(session_id)
            .cloned()
            .ok_or_else(|| SessionError::NotFound(session_id.to_string()))
    }

    async fn list(&self) -> Result<Vec<SessionMeta>, SessionError> {
        Ok(self
            .meta
            .read()
            .map_err(|e| SessionError::Storage(e.to_string()))?
            .values()
            .cloned()
            .collect())
    }
}

/// Create a new session and persist its meta. Returns the new session id.
pub async fn create_session<S: SessionStore + ?Sized>(
    store: &S,
    display_name: Option<String>,
    cwd: Option<String>,
) -> Result<String, SessionError> {
    let now = chrono::Utc::now().timestamp_millis();
    let session_id = Uuid::new_v4().to_string();
    store
        .create(SessionMeta {
            session_id: session_id.clone(),
            display_name,
            created_at: now,
            updated_at: now,
            cwd,
            branch_count: 1,
        })
        .await?;
    Ok(session_id)
}

/// Append a message entry, deriving id and timestamp.
pub async fn append_message<S: SessionStore + ?Sized>(
    store: &S,
    session_id: &str,
    parent_id: Option<String>,
    message: AgentMessage,
) -> Result<String, SessionError> {
    let id = Uuid::new_v4().to_string();
    let entry = SessionEntry::Message {
        id: id.clone(),
        parent_id,
        message,
        timestamp: chrono::Utc::now().timestamp_millis(),
    };
    store.append(session_id, entry).await?;
    Ok(id)
}

/// Active path from root to leaf. If `leaf` is None, walks back from the most
/// recently appended entry. Returns entry ids in root-first order.
pub async fn active_path<S: SessionStore + ?Sized>(
    store: &S,
    session_id: &str,
    leaf: Option<&str>,
) -> Result<Vec<String>, SessionError> {
    let entries = store.load_entries(session_id).await?;
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let by_id: HashMap<&str, &SessionEntry> = entries.iter().map(|e| (e.id(), e)).collect();
    let leaf_id = match leaf {
        Some(id) => id,
        None => entries.last().expect("non-empty checked").id(),
    };
    let mut path: Vec<String> = Vec::new();
    let mut cursor: Option<&str> = Some(leaf_id);
    while let Some(id) = cursor {
        path.push(id.to_string());
        cursor = by_id.get(id).and_then(|e| e.parent_id());
    }
    path.reverse();
    Ok(path)
}

/// Build an `AgentContext` from the active path's message entries.
pub async fn load_messages<S: SessionStore + ?Sized>(
    store: &S,
    session_id: &str,
    leaf: Option<&str>,
) -> Result<Vec<AgentMessage>, SessionError> {
    let entries = store.load_entries(session_id).await?;
    let path = active_path(store, session_id, leaf).await?;
    let by_id: HashMap<&str, &SessionEntry> = entries.iter().map(|e| (e.id(), e)).collect();
    let mut messages: Vec<AgentMessage> = Vec::new();
    for id in &path {
        if let Some(SessionEntry::Message { message, .. }) = by_id.get(id.as_str()).copied() {
            messages.push(message.clone());
        }
    }
    Ok(messages)
}

/// Hydrate an `AgentContext` from a session leaf using a system prompt.
pub async fn load_context<S: SessionStore + ?Sized>(
    store: &S,
    session_id: &str,
    leaf: Option<&str>,
    system_prompt: String,
) -> Result<AgentContext, SessionError> {
    let messages = load_messages(store, session_id, leaf).await?;
    Ok(AgentContext {
        system_prompt,
        messages,
        tools: Vec::new(),
    })
}

/// Threshold above which `fork` collapses the source path into a single
/// `BranchSummary` entry rather than copying every entry.
const FORK_SUMMARY_THRESHOLD: usize = 50;

/// Copies the active path up to `from_entry_id` into a new session.
///
/// If the source path has more than `FORK_SUMMARY_THRESHOLD` entries between
/// root and `from_entry_id`, a single `BranchSummary` entry is generated in the
/// new session in place of the copied entries. Otherwise every entry is
/// duplicated with re-mapped ids and parent links.
pub async fn fork<S: SessionStore + ?Sized>(
    store: &S,
    source_session_id: &str,
    from_entry_id: &str,
) -> Result<String, SessionError> {
    let entries = store.load_entries(source_session_id).await?;
    let by_id: HashMap<&str, &SessionEntry> = entries.iter().map(|e| (e.id(), e)).collect();
    if !by_id.contains_key(from_entry_id) {
        return Err(SessionError::EntryNotFound(from_entry_id.to_string()));
    }
    let path_ids = active_path(store, source_session_id, Some(from_entry_id)).await?;

    let source_meta = store.load_meta(source_session_id).await?;
    let new_session_id = create_session(
        store,
        source_meta.display_name.map(|n| format!("{n} (fork)")),
        source_meta.cwd,
    )
    .await?;

    if path_ids.len() > FORK_SUMMARY_THRESHOLD {
        let summary_id = Uuid::new_v4().to_string();
        let summary = format!(
            "Forked from session {source_session_id} at entry {from_entry_id}: {} entries collapsed.",
            path_ids.len()
        );
        let summary_entry = SessionEntry::BranchSummary {
            id: summary_id,
            parent_id: None,
            summary,
            from_id: from_entry_id.to_string(),
            timestamp: chrono::Utc::now().timestamp_millis(),
        };
        store.append(&new_session_id, summary_entry).await?;
        return Ok(new_session_id);
    }

    let mut id_map: HashMap<String, String> = HashMap::new();
    for old_id in &path_ids {
        let entry = by_id
            .get(old_id.as_str())
            .copied()
            .ok_or_else(|| SessionError::EntryNotFound(old_id.clone()))?
            .clone();
        let new_id = Uuid::new_v4().to_string();
        let new_parent = entry.parent_id().and_then(|p| id_map.get(p).cloned());
        let copied = entry.with_id(new_id.clone()).with_parent(new_parent);
        store.append(&new_session_id, copied).await?;
        id_map.insert(old_id.clone(), new_id);
    }

    Ok(new_session_id)
}

/// Full duplicate. Every entry copied with re-mapped ids; parent links rewired
/// to point at the new ids. Returns the new session id.
pub async fn clone_session<S: SessionStore + ?Sized>(
    store: &S,
    source_session_id: &str,
) -> Result<String, SessionError> {
    let entries = store.load_entries(source_session_id).await?;
    let source_meta = store.load_meta(source_session_id).await?;
    let new_session_id = create_session(
        store,
        source_meta.display_name.map(|n| format!("{n} (clone)")),
        source_meta.cwd,
    )
    .await?;

    let mut id_map: HashMap<String, String> = HashMap::new();
    for entry in &entries {
        let new_id = Uuid::new_v4().to_string();
        id_map.insert(entry.id().to_string(), new_id);
    }
    for entry in entries {
        let new_id = id_map
            .get(entry.id())
            .cloned()
            .expect("id_map populated for every entry");
        let new_parent = entry.parent_id().and_then(|p| id_map.get(p).cloned());
        let copied = entry.with_id(new_id).with_parent(new_parent);
        store.append(&new_session_id, copied).await?;
    }
    Ok(new_session_id)
}

/// Append a Compaction entry. Returns the new entry id. `parent_id` defaults
/// to the most recent entry on the active path when None.
pub async fn compact<S: SessionStore + ?Sized>(
    store: &S,
    session_id: &str,
    summary: String,
    file_ops: CompactionDetails,
    parent_id: Option<String>,
    tokens_before: u64,
) -> Result<String, SessionError> {
    let resolved_parent = match parent_id {
        Some(p) => Some(p),
        None => active_path(store, session_id, None).await?.last().cloned(),
    };
    let id = Uuid::new_v4().to_string();
    let entry = SessionEntry::Compaction {
        id: id.clone(),
        parent_id: resolved_parent,
        summary,
        tokens_before,
        details: file_ops,
        timestamp: chrono::Utc::now().timestamp_millis(),
    };
    store.append(session_id, entry).await?;
    Ok(id)
}

/// Returns the nested tree from the root of the session.
///
/// If multiple entries have `parent_id == None`, the first appended is treated
/// as the root and any subsequent root-less entries become its siblings under
/// a synthetic ordering — they appear as direct children of the first root in
/// append order. In well-formed sessions there is exactly one root entry.
pub async fn tree<S: SessionStore + ?Sized>(
    store: &S,
    session_id: &str,
) -> Result<TreeNode, SessionError> {
    let entries = store.load_entries(session_id).await?;
    if entries.is_empty() {
        return Err(SessionError::EntryNotFound(format!(
            "session {session_id} has no entries"
        )));
    }
    let mut children_by_parent: HashMap<Option<String>, Vec<SessionEntry>> = HashMap::new();
    for entry in entries {
        children_by_parent
            .entry(entry.parent_id().map(|s| s.to_string()))
            .or_default()
            .push(entry);
    }
    let mut roots = children_by_parent.remove(&None).unwrap_or_default();
    if roots.is_empty() {
        return Err(SessionError::EntryNotFound(format!(
            "session {session_id} has no root entry"
        )));
    }
    let root = roots.remove(0);
    let node = build_node(root, &mut children_by_parent);
    Ok(node)
}

fn build_node(
    entry: SessionEntry,
    by_parent: &mut HashMap<Option<String>, Vec<SessionEntry>>,
) -> TreeNode {
    let kids = by_parent
        .remove(&Some(entry.id().to_string()))
        .unwrap_or_default();
    let children = kids.into_iter().map(|c| build_node(c, by_parent)).collect();
    TreeNode { entry, children }
}

/// Returns a self-contained HTML document rendering the active path.
///
/// User messages are styled cyan, assistant white-on-dark, tool results dim,
/// thinking blocks italic. All CSS is inline. Special HTML characters in
/// content are escaped.
pub async fn export_html<S: SessionStore + ?Sized>(
    store: &S,
    session_id: &str,
    branch_leaf: Option<&str>,
) -> Result<String, SessionError> {
    let entries = store.load_entries(session_id).await?;
    let path = active_path(store, session_id, branch_leaf).await?;
    let by_id: HashMap<&str, &SessionEntry> = entries.iter().map(|e| (e.id(), e)).collect();

    let mut body = String::new();
    for id in &path {
        let Some(entry) = by_id.get(id.as_str()).copied() else {
            continue;
        };
        body.push_str(&render_entry_html(entry));
    }

    let title = html_escape(session_id);
    Ok(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>session {title}</title>
<style>
  body {{ background: #0d1117; color: #e6edf3; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", monospace; margin: 0; padding: 24px; }}
  .session {{ max-width: 920px; margin: 0 auto; }}
  .entry {{ padding: 12px 16px; margin: 12px 0; border-radius: 6px; border-left: 3px solid #30363d; white-space: pre-wrap; word-wrap: break-word; }}
  .role {{ font-weight: 600; font-size: 0.78em; letter-spacing: 0.08em; text-transform: uppercase; opacity: 0.7; margin-bottom: 6px; }}
  .user {{ background: #0b2730; border-left-color: #06b6d4; color: #67e8f9; }}
  .assistant {{ background: #161b22; border-left-color: #c9d1d9; color: #f0f6fc; }}
  .tool-result {{ background: #161b22; border-left-color: #6e7681; color: #8b949e; opacity: 0.85; }}
  .thinking {{ font-style: italic; opacity: 0.75; border-left-color: #a371f7; }}
  .custom {{ background: #161b22; border-left-color: #d29922; }}
  .summary {{ background: #1c1d23; border-left-color: #f0883e; }}
  .compaction {{ background: #1c1d23; border-left-color: #2ea043; }}
  pre {{ margin: 0; font-family: inherit; }}
</style>
</head>
<body>
<div class="session">
{body}</div>
</body>
</html>"#
    ))
}

fn render_entry_html(entry: &SessionEntry) -> String {
    match entry {
        SessionEntry::Message { message, .. } => render_message_html(message),
        SessionEntry::CustomMessage {
            custom_type,
            content,
            display,
            ..
        } => {
            let text = display
                .as_deref()
                .map_or_else(|| html_escape(&content.to_string()), html_escape);
            let kind = html_escape(custom_type);
            format!(
                "<div class=\"entry custom\"><div class=\"role\">custom · {kind}</div><pre>{text}</pre></div>\n"
            )
        }
        SessionEntry::BranchSummary {
            summary, from_id, ..
        } => {
            let s = html_escape(summary);
            let f = html_escape(from_id);
            format!(
                "<div class=\"entry summary\"><div class=\"role\">branch summary · from {f}</div><pre>{s}</pre></div>\n"
            )
        }
        SessionEntry::Compaction {
            summary,
            tokens_before,
            details,
            ..
        } => {
            let s = html_escape(summary);
            let read = details
                .read_files
                .iter()
                .map(|f| html_escape(f))
                .collect::<Vec<_>>()
                .join(", ");
            let modified = details
                .modified_files
                .iter()
                .map(|f| html_escape(f))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "<div class=\"entry compaction\"><div class=\"role\">compaction · {tokens_before} tokens before</div><pre>{s}\n\nread: {read}\nmodified: {modified}</pre></div>\n"
            )
        }
    }
}

fn render_message_html(message: &AgentMessage) -> String {
    match message {
        AgentMessage::User(u) => {
            let body = render_blocks_html(&u.content);
            format!("<div class=\"entry user\"><div class=\"role\">user</div>{body}</div>\n")
        }
        AgentMessage::Assistant(a) => {
            let body = render_blocks_html(&a.content);
            format!(
                "<div class=\"entry assistant\"><div class=\"role\">assistant · {}</div>{body}</div>\n",
                html_escape(&a.model)
            )
        }
        AgentMessage::ToolResult(tr) => {
            let body = render_blocks_html(&tr.content);
            let name = html_escape(&tr.tool_name);
            format!(
                "<div class=\"entry tool-result\"><div class=\"role\">tool result · {name}</div>{body}</div>\n"
            )
        }
        AgentMessage::Custom(c) => {
            let kind = html_escape(&c.custom_type);
            let text = c
                .display
                .as_deref()
                .map_or_else(|| html_escape(&c.content.to_string()), html_escape);
            format!(
                "<div class=\"entry custom\"><div class=\"role\">custom · {kind}</div><pre>{text}</pre></div>\n"
            )
        }
    }
}

fn render_blocks_html(blocks: &[ContentBlock]) -> String {
    let mut out = String::new();
    for block in blocks {
        match block {
            ContentBlock::Text(t) => {
                out.push_str("<pre>");
                out.push_str(&html_escape(&t.text));
                out.push_str("</pre>");
            }
            ContentBlock::Thinking { text, .. } => {
                out.push_str("<div class=\"thinking\"><pre>");
                out.push_str(&html_escape(text));
                out.push_str("</pre></div>");
            }
            ContentBlock::Image(img) => {
                out.push_str("<pre>[image: ");
                out.push_str(&html_escape(&img.mime));
                out.push_str("]</pre>");
            }
            ContentBlock::ToolCall {
                name, arguments, ..
            } => {
                out.push_str("<pre>tool call: ");
                out.push_str(&html_escape(name));
                out.push(' ');
                out.push_str(&html_escape(&arguments.to_string()));
                out.push_str("</pre>");
            }
            ContentBlock::ToolResult { content, .. } => {
                out.push_str(&render_blocks_html(content));
            }
        }
    }
    out
}

/// Escape the four HTML special characters plus the apostrophe.
fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Registered function ids exposed by [`register_with_iii`].
pub mod function_ids {
    pub const FORK: &str = "session::fork";
    pub const CLONE: &str = "session::clone";
    pub const COMPACT: &str = "session::compact";
    pub const TREE: &str = "session::tree";
    pub const EXPORT_HTML: &str = "session::export_html";
    pub const CREATE: &str = "session::create";
    pub const APPEND: &str = "session::append";
    pub const MESSAGES: &str = "session::messages";
}

/// Register the five `session::*` iii functions on `iii`, backed by `store`.
///
/// Returns a [`SessionFunctionRefs`] handle. Drop or call
/// [`SessionFunctionRefs::unregister_all`] to deregister everything in one
/// shot. The Rust API ([`fork`], [`clone_session`], [`compact`], [`tree`],
/// [`export_html`]) remains available for in-process callers; the iii
/// functions are thin wrappers over the same impls.
///
/// # Payload shapes
///
/// - `session::fork` — `{ "source_session_id": str, "from_entry_id": str }`
///   → `{ "session_id": str }`
/// - `session::clone` — `{ "source_session_id": str }`
///   → `{ "session_id": str }`
/// - `session::compact` — `{ "session_id": str, "summary": str,
///   "tokens_before": u64, "details": { "read_files": [str],
///   "modified_files": [str] }, "parent_id": str? }`
///   → `{ "entry_id": str }`
/// - `session::tree` — `{ "session_id": str }` → `TreeNode`
/// - `session::export_html` — `{ "session_id": str, "branch_leaf": str? }`
///   → `{ "html": str }`
pub fn register_with_iii<S>(iii: &iii_sdk::III, store: std::sync::Arc<S>) -> SessionFunctionRefs
where
    S: SessionStore + Send + Sync + 'static,
{
    use iii_sdk::{IIIError, RegisterFunctionMessage};
    use serde_json::json;

    let mut refs: Vec<iii_sdk::FunctionRef> = Vec::with_capacity(5);

    let store_fork = store.clone();
    refs.push(
        iii.register_function((
            RegisterFunctionMessage::with_id(function_ids::FORK.into())
                .with_description("Fork a session at a given entry into a new session id".into()),
            move |payload: serde_json::Value| {
                let store = store_fork.clone();
                async move {
                    let source = required_str(&payload, "source_session_id")?;
                    let from = required_str(&payload, "from_entry_id")?;
                    let new_id = fork(store.as_ref(), &source, &from)
                        .await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    Ok(json!({ "session_id": new_id }))
                }
            },
        )),
    );

    let store_clone = store.clone();
    refs.push(
        iii.register_function((
            RegisterFunctionMessage::with_id(function_ids::CLONE.into())
                .with_description("Duplicate a session with re-mapped ids".into()),
            move |payload: serde_json::Value| {
                let store = store_clone.clone();
                async move {
                    let source = required_str(&payload, "source_session_id")?;
                    let new_id = clone_session(store.as_ref(), &source)
                        .await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    Ok(json!({ "session_id": new_id }))
                }
            },
        )),
    );

    let store_compact = store.clone();
    refs.push(
        iii.register_function((
            RegisterFunctionMessage::with_id(function_ids::COMPACT.into())
                .with_description("Append a Compaction entry summarising the active path".into()),
            move |payload: serde_json::Value| {
                let store = store_compact.clone();
                async move {
                    let session_id = required_str(&payload, "session_id")?;
                    let summary = required_str(&payload, "summary")?;
                    let tokens_before = payload
                        .get("tokens_before")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let details: CompactionDetails = payload
                        .get("details")
                        .cloned()
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(|e| IIIError::Handler(e.to_string()))?
                        .unwrap_or_default();
                    let parent_id = payload
                        .get("parent_id")
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string);
                    let entry_id = compact(
                        store.as_ref(),
                        &session_id,
                        summary,
                        details,
                        parent_id,
                        tokens_before,
                    )
                    .await
                    .map_err(|e| IIIError::Handler(e.to_string()))?;
                    Ok(json!({ "entry_id": entry_id }))
                }
            },
        )),
    );

    let store_tree = store.clone();
    refs.push(
        iii.register_function((
            RegisterFunctionMessage::with_id(function_ids::TREE.into())
                .with_description("Return the session tree as a nested TreeNode".into()),
            move |payload: serde_json::Value| {
                let store = store_tree.clone();
                async move {
                    let session_id = required_str(&payload, "session_id")?;
                    let node = tree(store.as_ref(), &session_id)
                        .await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    serde_json::to_value(node).map_err(|e| IIIError::Handler(e.to_string()))
                }
            },
        )),
    );

    let store_html = store.clone();
    refs.push(iii.register_function(
        (
            RegisterFunctionMessage::with_id(function_ids::EXPORT_HTML.into()).with_description(
                "Render the active path as a self-contained HTML document".into(),
            ),
            move |payload: serde_json::Value| {
                let store = store_html.clone();
                async move {
                    let session_id = required_str(&payload, "session_id")?;
                    let leaf = payload
                        .get("branch_leaf")
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string);
                    let html = export_html(store.as_ref(), &session_id, leaf.as_deref())
                        .await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    Ok(json!({ "html": html }))
                }
            },
        ),
    ));

    let store_create = store_for_create(&store);
    refs.push(
        iii.register_function((
            RegisterFunctionMessage::with_id(function_ids::CREATE.into())
                .with_description("Create a new empty session record".into()),
            move |payload: serde_json::Value| {
                let store = store_create.clone();
                async move {
                    let display_name = payload
                        .get("display_name")
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string);
                    let cwd = payload
                        .get("cwd")
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string);
                    let new_id = create_session(store.as_ref(), display_name, cwd)
                        .await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    Ok(json!({ "session_id": new_id }))
                }
            },
        )),
    );

    let store_append = store_for_append(&store);
    refs.push(
        iii.register_function((
            RegisterFunctionMessage::with_id(function_ids::APPEND.into())
                .with_description("Append an AgentMessage entry to a session".into()),
            move |payload: serde_json::Value| {
                let store = store_append.clone();
                async move {
                    let session_id = required_str(&payload, "session_id")?;
                    let parent_id = payload
                        .get("parent_id")
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string);
                    let message_value = payload.get("message").cloned().ok_or_else(|| {
                        IIIError::Handler("missing required field: message".into())
                    })?;
                    let message: AgentMessage = serde_json::from_value(message_value)
                        .map_err(|e| IIIError::Handler(format!("invalid message: {e}")))?;
                    let entry_id = append_message(store.as_ref(), &session_id, parent_id, message)
                        .await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    Ok(json!({ "entry_id": entry_id }))
                }
            },
        )),
    );

    let store_messages = store;
    refs.push(iii.register_function((
        RegisterFunctionMessage::with_id(function_ids::MESSAGES.into()).with_description(
            "Load every AgentMessage on the active path of a session, oldest first".into(),
        ),
        move |payload: serde_json::Value| {
            let store = store_messages.clone();
            async move {
                let session_id = required_str(&payload, "session_id")?;
                let leaf = payload
                    .get("branch_leaf")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string);
                let messages = load_messages(store.as_ref(), &session_id, leaf.as_deref())
                    .await
                    .map_err(|e| IIIError::Handler(e.to_string()))?;
                Ok(json!({ "messages": messages }))
            }
        },
    )));

    SessionFunctionRefs { refs }
}

fn store_for_create<S>(s: &std::sync::Arc<S>) -> std::sync::Arc<S>
where
    S: ?Sized,
{
    s.clone()
}
fn store_for_append<S>(s: &std::sync::Arc<S>) -> std::sync::Arc<S>
where
    S: ?Sized,
{
    s.clone()
}

/// Handle returned by [`register_with_iii`]. Drop or call
/// [`unregister_all`](Self::unregister_all) to remove every registered function.
pub struct SessionFunctionRefs {
    refs: Vec<iii_sdk::FunctionRef>,
}

impl SessionFunctionRefs {
    /// Unregister every `session::*` function this batch installed.
    pub fn unregister_all(self) {
        for f in self.refs {
            f.unregister();
        }
    }

    /// Number of functions registered.
    pub fn len(&self) -> usize {
        self.refs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.refs.is_empty()
    }
}

fn required_str(payload: &serde_json::Value, field: &str) -> Result<String, iii_sdk::IIIError> {
    payload
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| iii_sdk::IIIError::Handler(format!("missing required field: {field}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_types::{ContentBlock, TextContent, UserMessage};

    fn user(text: &str, ts: i64) -> AgentMessage {
        AgentMessage::User(UserMessage {
            content: vec![ContentBlock::Text(TextContent {
                text: text.to_string(),
            })],
            timestamp: ts,
        })
    }

    #[tokio::test]
    async fn create_then_load_meta() {
        let store = InMemoryStore::new();
        let id = create_session(&store, Some("test".into()), None)
            .await
            .unwrap();
        let meta = store.load_meta(&id).await.unwrap();
        assert_eq!(meta.session_id, id);
        assert_eq!(meta.display_name, Some("test".into()));
    }

    #[tokio::test]
    async fn append_and_active_path_linear() {
        let store = InMemoryStore::new();
        let id = create_session(&store, None, None).await.unwrap();
        let a = append_message(&store, &id, None, user("a", 1))
            .await
            .unwrap();
        let b = append_message(&store, &id, Some(a.clone()), user("b", 2))
            .await
            .unwrap();
        let c = append_message(&store, &id, Some(b.clone()), user("c", 3))
            .await
            .unwrap();
        let path = active_path(&store, &id, None).await.unwrap();
        assert_eq!(path, vec![a, b, c]);
    }

    #[tokio::test]
    async fn active_path_at_leaf_branches() {
        let store = InMemoryStore::new();
        let id = create_session(&store, None, None).await.unwrap();
        let a = append_message(&store, &id, None, user("a", 1))
            .await
            .unwrap();
        let b = append_message(&store, &id, Some(a.clone()), user("b", 2))
            .await
            .unwrap();
        let c = append_message(&store, &id, Some(a.clone()), user("c", 3))
            .await
            .unwrap();
        let path_b = active_path(&store, &id, Some(&b)).await.unwrap();
        let path_c = active_path(&store, &id, Some(&c)).await.unwrap();
        assert_eq!(path_b, vec![a.clone(), b]);
        assert_eq!(path_c, vec![a, c]);
    }

    #[tokio::test]
    async fn load_messages_returns_path_messages() {
        let store = InMemoryStore::new();
        let id = create_session(&store, None, None).await.unwrap();
        append_message(&store, &id, None, user("hello", 1))
            .await
            .unwrap();
        let msgs = load_messages(&store, &id, None).await.unwrap();
        assert_eq!(msgs.len(), 1);
    }

    #[tokio::test]
    async fn list_returns_all_sessions() {
        let store = InMemoryStore::new();
        create_session(&store, Some("a".into()), None)
            .await
            .unwrap();
        create_session(&store, Some("b".into()), None)
            .await
            .unwrap();
        let listed = store.list().await.unwrap();
        assert_eq!(listed.len(), 2);
    }

    async fn build_linear_session(store: &InMemoryStore, len: usize) -> (String, Vec<String>) {
        let id = create_session(store, Some("linear".into()), None)
            .await
            .unwrap();
        let mut ids: Vec<String> = Vec::with_capacity(len);
        let mut parent: Option<String> = None;
        for i in 0..len {
            let msg = user(&format!("m{i}"), i as i64);
            let new_id = append_message(store, &id, parent.clone(), msg)
                .await
                .unwrap();
            ids.push(new_id.clone());
            parent = Some(new_id);
        }
        (id, ids)
    }

    #[tokio::test]
    async fn fork_preserves_source_path_messages() {
        let store = InMemoryStore::new();
        let (src, ids) = build_linear_session(&store, 5).await;
        let from = ids[3].clone();
        let new_id = fork(&store, &src, &from).await.unwrap();
        let path = active_path(&store, &new_id, None).await.unwrap();
        assert_eq!(path.len(), 4, "should have copied 4 entries");
        let msgs = load_messages(&store, &new_id, None).await.unwrap();
        assert_eq!(msgs.len(), 4);
        let entries = store.load_entries(&new_id).await.unwrap();
        for entry in &entries {
            assert!(
                !ids.contains(&entry.id().to_string()),
                "ids must be re-mapped"
            );
        }
    }

    #[tokio::test]
    async fn fork_generates_branch_summary_when_over_threshold() {
        let store = InMemoryStore::new();
        let (src, ids) = build_linear_session(&store, FORK_SUMMARY_THRESHOLD + 5).await;
        let from = ids.last().unwrap().clone();
        let new_id = fork(&store, &src, &from).await.unwrap();
        let entries = store.load_entries(&new_id).await.unwrap();
        assert_eq!(entries.len(), 1);
        match &entries[0] {
            SessionEntry::BranchSummary { from_id, .. } => {
                assert_eq!(from_id, &from);
            }
            other => panic!("expected BranchSummary, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fork_unknown_entry_errors() {
        let store = InMemoryStore::new();
        let (src, _) = build_linear_session(&store, 3).await;
        let err = fork(&store, &src, "no-such-id").await.unwrap_err();
        assert!(matches!(err, SessionError::EntryNotFound(_)));
    }

    #[tokio::test]
    async fn clone_duplicates_entries_with_remapped_ids() {
        let store = InMemoryStore::new();
        let (src, ids) = build_linear_session(&store, 4).await;
        let new_id = clone_session(&store, &src).await.unwrap();
        assert_ne!(new_id, src);
        let new_entries = store.load_entries(&new_id).await.unwrap();
        assert_eq!(new_entries.len(), ids.len());
        for entry in &new_entries {
            assert!(!ids.contains(&entry.id().to_string()));
        }
        let path = active_path(&store, &new_id, None).await.unwrap();
        assert_eq!(path.len(), 4);
        for i in 1..new_entries.len() {
            let parent = new_entries[i].parent_id().unwrap();
            assert_eq!(parent, new_entries[i - 1].id());
        }
    }

    #[tokio::test]
    async fn clone_yields_fresh_session_id() {
        let store = InMemoryStore::new();
        let (src, _) = build_linear_session(&store, 2).await;
        let a = clone_session(&store, &src).await.unwrap();
        let b = clone_session(&store, &src).await.unwrap();
        assert_ne!(a, src);
        assert_ne!(b, src);
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn compact_appends_compaction_entry_with_details() {
        let store = InMemoryStore::new();
        let (src, _) = build_linear_session(&store, 3).await;
        let details = CompactionDetails {
            read_files: vec!["src/lib.rs".into()],
            modified_files: vec!["README.md".into()],
        };
        let entry_id = compact(
            &store,
            &src,
            "summary text".into(),
            details.clone(),
            None,
            12345,
        )
        .await
        .unwrap();
        let entries = store.load_entries(&src).await.unwrap();
        let last = entries.last().unwrap();
        match last {
            SessionEntry::Compaction {
                id,
                summary,
                tokens_before,
                details: d,
                ..
            } => {
                assert_eq!(id, &entry_id);
                assert_eq!(summary, "summary text");
                assert_eq!(*tokens_before, 12345);
                assert_eq!(d, &details);
            }
            other => panic!("expected Compaction, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tree_returns_nested_structure_with_correct_child_counts() {
        let store = InMemoryStore::new();
        let id = create_session(&store, None, None).await.unwrap();
        let a = append_message(&store, &id, None, user("a", 1))
            .await
            .unwrap();
        let b = append_message(&store, &id, Some(a.clone()), user("b", 2))
            .await
            .unwrap();
        let _c = append_message(&store, &id, Some(a.clone()), user("c", 3))
            .await
            .unwrap();
        let _d = append_message(&store, &id, Some(b.clone()), user("d", 4))
            .await
            .unwrap();

        let root = tree(&store, &id).await.unwrap();
        assert_eq!(root.entry.id(), a);
        assert_eq!(root.children.len(), 2);
        let total: usize = root
            .children
            .iter()
            .map(|n| 1 + n.children.len())
            .sum::<usize>()
            + 1;
        assert_eq!(total, 4);
    }

    #[tokio::test]
    async fn tree_empty_session_errors() {
        let store = InMemoryStore::new();
        let id = create_session(&store, None, None).await.unwrap();
        let err = tree(&store, &id).await.unwrap_err();
        assert!(matches!(err, SessionError::EntryNotFound(_)));
    }

    #[tokio::test]
    async fn export_html_escapes_html_special_chars() {
        let store = InMemoryStore::new();
        let id = create_session(&store, None, None).await.unwrap();
        append_message(&store, &id, None, user("<script>alert(\"x\")</script>", 1))
            .await
            .unwrap();
        let html = export_html(&store, &id, None).await.unwrap();
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&quot;"));
    }

    #[tokio::test]
    async fn export_html_includes_all_messages_on_active_path() {
        let store = InMemoryStore::new();
        let id = create_session(&store, None, None).await.unwrap();
        let a = append_message(&store, &id, None, user("alpha", 1))
            .await
            .unwrap();
        let b = append_message(&store, &id, Some(a.clone()), user("beta", 2))
            .await
            .unwrap();
        let _branch = append_message(&store, &id, Some(a.clone()), user("offshoot", 3))
            .await
            .unwrap();
        let html = export_html(&store, &id, Some(&b)).await.unwrap();
        assert!(html.contains("alpha"));
        assert!(html.contains("beta"));
        assert!(!html.contains("offshoot"));
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("entry user"));
    }
}
