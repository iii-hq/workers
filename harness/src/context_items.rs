//! `harness::context-items` (chat-projects D10): what a conversation has
//! gathered, for the context pane every client draws the same way —
//! subagents, pull requests, links, attachments and images, newest first.
//!
//! The rules are ported from the ADE's `chat-aside.ts` (`collectChatLinks`,
//! `parsePullRequestUrl`, `collectChatFiles`, `collectChatImages`) and its
//! tests moved with them (below). The transcript is read in pages through
//! `session::messages-range`; an in-memory index per session, keyed by
//! `message_count` (LRU of 64 sessions), keeps what each entry contributed, so
//! a later call scans only the entries added since (plus the last one, which
//! may have been streaming).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::clients::session::{SessionClient, SessionSummary};
use crate::deps::Deps;
use crate::error::HarnessError;

pub const CONTEXT_ITEMS_ID: &str = "harness::context-items";
pub const CONTEXT_ITEMS_DESC: &str = "Internal control-plane: what a conversation gathered, newest first — its subagents, \
     pull requests (with their state when the github worker is present), links, attachments and \
     images sent and received, and its side chats — for the context pane.";

pub const MAX_LINKS: usize = 200;
pub const MAX_ATTACHMENTS: usize = 100;
pub const MAX_IMAGES: usize = 200;
/// Sessions whose index stays in memory.
const INDEX_SESSIONS: usize = 64;
/// Transcript page size for `session::messages-range`.
const PAGE: u64 = 200;
/// How long one PR's reading stays fresh.
const PR_TTL: Duration = Duration::from_secs(120);
/// The most one call waits for PR readings.
const PR_BUDGET: Duration = Duration::from_secs(3);
/// PRs read per call (the newest).
const PR_READS: usize = 20;
/// Descendants counted per subagent before stopping.
const DESCENDANTS_CAP: usize = 1000;
/// Session metadata key: the PRs the user dismissed from the pane (the ADE's).
pub const DISMISSED_PRS_KEY: &str = "context_dismissed_prs";

// ── wire shapes ──

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ContextItemsRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SubagentItem {
    pub session_id: String,
    pub title: String,
    /// `running`, `done` or `failed`.
    pub status: String,
    /// The `subagent_display` label (for example "Explore").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<i64>,
    /// Sessions below this one (its own subagents and theirs).
    pub descendants: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PullRequestItem {
    /// `owner/name`, lower case.
    pub repo: String,
    pub number: u64,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `open`, `draft`, `merged` or `closed`; absent without the github worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// `passing`, `failing` or `pending`; absent without checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<String>,
    /// The newest entry naming it.
    pub entry_id: String,
    /// `created` (a `github::pr::create` result) or `mentioned`.
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LinkItem {
    pub url: String,
    pub host: String,
    /// The text of a Markdown `[text](url)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The newest entry naming it.
    pub entry_id: String,
    /// Mentions in the conversation.
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AttachmentItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    pub name: String,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// `sent` (in a user message) or `received` (in a function result or an
    /// assistant message, or a file the agent wrote).
    pub direction: String,
    pub entry_id: String,
    /// For a file the agent wrote: its absolute path on the engine's computer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImageItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    pub mime: String,
    /// `sent` or `received`, by the attachments' rule.
    pub direction: String,
    pub entry_id: String,
    /// The `image` block's index in the entry's message content.
    pub block_index: u64,
}

/// One side chat of the conversation (chat-projects phase 2): the pane row
/// "<title> · 6 messages · 10m", with a dot while a turn runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SideChatItem {
    pub session_id: String,
    /// "Side chat" until its first message names it.
    pub title: String,
    /// Messages added in the side chat since the fork (user, assistant and
    /// function results; 0 while nothing was sent). A side chat forked
    /// before phase 2 records no fork point: all its messages count.
    pub message_count: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// The conversation's entry it was forked at (`side_chat_fork_point`),
    /// for the mark under that message; absent for a side chat forked
    /// before phase 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_entry_id: Option<String>,
    /// `running`, `done` or `failed`, from its session status (as subagents).
    pub status: String,
    /// A turn is running in it (`status == "running"`).
    pub running: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Truncated {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ContextItemsResponse {
    pub session_id: String,
    pub message_count: u64,
    pub generated_at_ms: i64,
    pub subagents: Vec<SubagentItem>,
    pub pull_requests: Vec<PullRequestItem>,
    pub links: Vec<LinkItem>,
    pub attachments: Vec<AttachmentItem>,
    pub images: Vec<ImageItem>,
    pub side_chats: Vec<SideChatItem>,
    pub truncated: Truncated,
}

// ── per-entry extraction (pure) ──

static URL_IN_TEXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"https?://[^\s<>()\[\]{}"'`|\\]+"#).expect("url regex"));
static TRAILING_PUNCTUATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[.,;:!?*_~)'"]+$"#).expect("trailing regex"));
static PR_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https://github\.com/([\w.-]+/[\w.-]+)/pull/(\d+)").expect("pr regex")
});
static MARKDOWN_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[([^\]\n]+)\]\((https?://[^)\s]+)\)").expect("markdown link regex")
});

/// A PR named in the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestRef {
    pub repo: String,
    pub number: u64,
    pub url: String,
}

impl PullRequestRef {
    pub fn key(&self) -> String {
        format!("{}#{}", self.repo, self.number)
    }
}

/// `parsePullRequestUrl`: the PR a GitHub URL names, its repo lower-cased
/// (GitHub names are case-insensitive: one spelling keeps one row) and its
/// URL canonical.
pub fn parse_pull_request_url(url: &str) -> Option<PullRequestRef> {
    let captures = PR_URL.captures(url)?;
    let number: u64 = captures[2].parse().ok()?;
    let repo = captures[1].to_lowercase();
    Some(PullRequestRef {
        url: format!("https://github.com/{repo}/pull/{number}"),
        repo,
        number,
    })
}

/// The http(s) URLs in a text, trailing punctuation trimmed.
pub fn urls_in(text: &str) -> Vec<String> {
    URL_IN_TEXT
        .find_iter(text)
        .map(|found| {
            TRAILING_PUNCTUATION
                .replace(found.as_str(), "")
                .into_owned()
        })
        .collect()
}

/// `[text](url)` titles in a text, by URL.
fn markdown_titles(text: &str) -> HashMap<String, String> {
    MARKDOWN_LINK
        .captures_iter(text)
        .filter_map(|captures| {
            let title = captures[1].trim().to_string();
            let url = TRAILING_PUNCTUATION.replace(&captures[2], "").into_owned();
            (!title.is_empty() && title != url).then_some((url, title))
        })
        .collect()
}

/// The host of an http(s) URL (with its port), or `None`.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    (!host.is_empty()).then(|| host.to_lowercase())
}

/// One link mention.
#[derive(Debug, Clone, PartialEq)]
struct LinkHit {
    url: String,
    title: Option<String>,
}

/// One PR mention.
#[derive(Debug, Clone, PartialEq)]
struct PrHit {
    pr: PullRequestRef,
    created: bool,
}

/// A file the agent wrote with a write call, waiting for its result.
#[derive(Debug, Clone, PartialEq)]
struct WrittenFile {
    path: String,
    size: Option<u64>,
}

/// What one transcript entry contributes.
#[derive(Debug, Clone, Default, PartialEq)]
struct EntryItems {
    entry_id: String,
    links: Vec<LinkHit>,
    prs: Vec<PrHit>,
    attachments: Vec<AttachmentItem>,
    images: Vec<ImageItem>,
}

impl EntryItems {
    fn is_empty(&self) -> bool {
        self.links.is_empty()
            && self.prs.is_empty()
            && self.attachments.is_empty()
            && self.images.is_empty()
    }
}

/// Host writes only (`collectChatFiles`): a sandbox's paths do not open on
/// the engine's computer.
fn is_write_call(function_id: &str) -> bool {
    matches!(function_id, "shell::fs::write" | "fs::write")
}

const PR_CREATE: &str = "github::pr::create";

/// The real target and arguments of a call block (an `agent_call` wrapper
/// names its target in `function` and its arguments in `payload`).
fn call_target(function_id: &str, arguments: &Value) -> (String, Value) {
    if crate::policy::is_agent_call_name(function_id)
        && let Some(target) = arguments.get("function").and_then(Value::as_str)
    {
        let payload = arguments.get("payload").cloned().unwrap_or(Value::Null);
        let payload = match payload {
            Value::String(text) => serde_json::from_str(&text).unwrap_or(Value::Null),
            other => other,
        };
        return (target.to_string(), payload);
    }
    (function_id.to_string(), arguments.clone())
}

fn written_files(arguments: &Value) -> Vec<WrittenFile> {
    let entries: Vec<&Value> = match arguments.get("files").and_then(Value::as_array) {
        Some(files) => files.iter().collect(),
        None => vec![arguments],
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let path = entry.get("path")?.as_str()?;
            path.starts_with('/').then(|| WrittenFile {
                path: path.to_string(),
                size: entry
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| content.len() as u64),
            })
        })
        .collect()
}

/// A MIME type from a file name, for files the agent wrote.
pub fn mime_of(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "md" | "markdown" => "text/markdown",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "js" | "mjs" => "text/javascript",
        "ts" | "tsx" => "text/typescript",
        "rs" => "text/x-rust",
        "py" => "text/x-python",
        "sh" => "text/x-shellscript",
        _ => "application/octet-stream",
    }
}

fn block_text(block: &Value) -> Option<&str> {
    (block.get("type")?.as_str()? == "text")
        .then(|| block.get("text")?.as_str())
        .flatten()
}

/// Extract one transcript item (a `session::messages-range` item). `pending`
/// carries write calls from an assistant entry to their results.
fn scan_entry(item: &Value, pending: &mut HashMap<String, Vec<WrittenFile>>) -> EntryItems {
    let entry_id = item
        .get("entry_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut out = EntryItems {
        entry_id: entry_id.clone(),
        ..EntryItems::default()
    };
    let Some(message) = item.get("message") else {
        return out;
    };
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let empty = Vec::new();
    let content = message
        .get("content")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let direction = if role == "user" { "sent" } else { "received" };

    // Text: links and PRs in what was said (user and assistant only).
    if role == "user" || role == "assistant" {
        for block in content {
            let Some(text) = block_text(block) else {
                continue;
            };
            let titles = markdown_titles(text);
            for url in urls_in(text) {
                match parse_pull_request_url(&url) {
                    Some(pr) => out.prs.push(PrHit { pr, created: false }),
                    None => out.links.push(LinkHit {
                        title: titles.get(&url).cloned(),
                        url,
                    }),
                }
            }
        }
    }

    // Files and images, sent or received, by block.
    if role == "user" || role == "assistant" || role == "function_result" {
        for (index, block) in content.iter().enumerate() {
            match block.get("type").and_then(Value::as_str) {
                Some("file") => {
                    let attachment_id = block
                        .get("attachment_id")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("file")
                        .to_string();
                    let mime = block
                        .get("mime")
                        .and_then(Value::as_str)
                        .unwrap_or_else(|| mime_of(&name))
                        .to_string();
                    out.attachments.push(AttachmentItem {
                        attachment_id,
                        name,
                        mime,
                        size: block.get("size").and_then(Value::as_u64),
                        direction: direction.into(),
                        entry_id: entry_id.clone(),
                        path: None,
                    });
                }
                Some("image") => out.images.push(ImageItem {
                    attachment_id: block
                        .get("attachment_id")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    mime: block
                        .get("mime")
                        .and_then(Value::as_str)
                        .unwrap_or("image/png")
                        .to_string(),
                    direction: direction.into(),
                    entry_id: entry_id.clone(),
                    block_index: index as u64,
                }),
                _ => {}
            }
        }
    }

    match role {
        "assistant" => {
            // Write calls wait for their results; call arguments are never
            // scanned for links.
            for block in content {
                if block.get("type").and_then(Value::as_str) != Some("function_call") {
                    continue;
                }
                let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
                let function_id = block
                    .get("function_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let arguments = block.get("arguments").cloned().unwrap_or(Value::Null);
                let (target, payload) = call_target(function_id, &arguments);
                if is_write_call(&target) {
                    let files = written_files(&payload);
                    if !files.is_empty() {
                        pending.insert(id.to_string(), files);
                    }
                }
            }
        }
        "function_result" => {
            let function_id = message
                .get("function_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let is_error = message.get("is_error").and_then(Value::as_bool) == Some(true);
            // PRs in any result: created by `github::pr::create`, else mentioned.
            let mut text = content
                .iter()
                .filter_map(block_text)
                .collect::<Vec<_>>()
                .join("\n");
            if let Some(details) = message.get("details").filter(|d| !d.is_null()) {
                text.push('\n');
                text.push_str(&details.to_string());
            }
            let created = function_id == PR_CREATE && !is_error;
            let mut seen = HashSet::new();
            for url in urls_in(&text) {
                if let Some(pr) = parse_pull_request_url(&url)
                    && seen.insert(pr.key())
                {
                    out.prs.push(PrHit { pr, created });
                }
            }
            let call_id = message
                .get("function_call_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if let Some(files) = pending.remove(call_id)
                && !is_error
            {
                for file in files {
                    let name = file
                        .path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&file.path)
                        .to_string();
                    out.attachments.push(AttachmentItem {
                        attachment_id: None,
                        mime: mime_of(&name).to_string(),
                        name,
                        size: file.size,
                        direction: "received".into(),
                        entry_id: entry_id.clone(),
                        path: Some(file.path),
                    });
                }
            }
        }
        _ => {}
    }
    out
}

// ── the index ──

/// What a session's transcript contributed, entry by entry.
#[derive(Debug, Clone, Default, PartialEq)]
struct SessionIndex {
    /// `message_count` / `updated_at` of the session when scanned.
    message_count: u64,
    updated_at: i64,
    scanned: bool,
    /// The newest entry scanned; the next scan starts there again (it may
    /// have been streaming).
    last_entry_id: Option<String>,
    /// Entries that contributed something, oldest first.
    entries: Vec<EntryItems>,
    /// Write calls still waiting for their results.
    pending: HashMap<String, Vec<WrittenFile>>,
    /// `pending` as it was before `last_entry_id` was scanned.
    pending_before_last: HashMap<String, Vec<WrittenFile>>,
}

impl SessionIndex {
    /// Scan `items` (path order). With `rescan_last`, the first item is
    /// the entry scanned last time: what it contributed is replaced.
    fn apply(&mut self, items: &[Value], rescan_last: bool) {
        let mut items = items.iter().peekable();
        if rescan_last
            && let Some(last) = self.last_entry_id.clone()
            && items
                .peek()
                .and_then(|item| item.get("entry_id"))
                .and_then(Value::as_str)
                == Some(last.as_str())
        {
            self.entries.retain(|entry| entry.entry_id != last);
            self.pending = self.pending_before_last.clone();
        }
        for item in items {
            self.pending_before_last = self.pending.clone();
            let scanned = scan_entry(item, &mut self.pending);
            if let Some(id) = item.get("entry_id").and_then(Value::as_str) {
                self.last_entry_id = Some(id.to_string());
            }
            if !scanned.is_empty() {
                self.entries.push(scanned);
            }
        }
        self.scanned = true;
    }
}

/// The in-memory indexes, an LRU of [`INDEX_SESSIONS`] sessions.
#[derive(Clone, Default)]
pub struct ContextIndex {
    inner: Arc<Mutex<IndexCache>>,
}

#[derive(Default)]
struct IndexCache {
    map: HashMap<String, SessionIndex>,
    order: VecDeque<String>,
}

impl ContextIndex {
    fn take(&self, session_id: &str) -> SessionIndex {
        let cache = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        cache.map.get(session_id).cloned().unwrap_or_default()
    }

    fn put(&self, session_id: &str, index: SessionIndex) {
        let mut cache = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        cache.order.retain(|id| id != session_id);
        cache.order.push_back(session_id.to_string());
        cache.map.insert(session_id.to_string(), index);
        while cache.order.len() > INDEX_SESSIONS {
            if let Some(evicted) = cache.order.pop_front() {
                cache.map.remove(&evicted);
            }
        }
    }

    /// Sessions indexed now (tests).
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .map
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The lists, newest first, from an index and the session's metadata (its
/// dismissed PRs and working folder).
fn aggregate(
    index: &SessionIndex,
    metadata: Option<&serde_json::Map<String, Value>>,
) -> (
    Vec<PullRequestItem>,
    Vec<LinkItem>,
    Vec<AttachmentItem>,
    Vec<ImageItem>,
    Truncated,
) {
    let dismissed: HashSet<String> = metadata
        .and_then(|m| m.get(DISMISSED_PRS_KEY))
        .and_then(Value::as_array)
        .map(|keys| {
            keys.iter()
                .filter_map(Value::as_str)
                .map(str::to_lowercase)
                .collect()
        })
        .unwrap_or_default();
    // A write inside the working folder is among the folder's own changes.
    let inside = metadata
        .and_then(|m| m.get(crate::types::turn::FS_SCOPE_KEY))
        .and_then(|scope| scope.get("root"))
        .and_then(Value::as_str)
        .filter(|root| !root.is_empty())
        .map(|root| format!("{}/", root.trim_end_matches('/')));

    let mut prs: Vec<PullRequestItem> = Vec::new();
    let mut pr_at: HashMap<String, usize> = HashMap::new();
    let mut links: Vec<LinkItem> = Vec::new();
    let mut link_at: HashMap<String, usize> = HashMap::new();
    let mut attachments: Vec<AttachmentItem> = Vec::new();
    let mut attachment_seen: HashSet<String> = HashSet::new();
    let mut images: Vec<ImageItem> = Vec::new();
    let mut truncated = Truncated::default();

    for entry in index.entries.iter().rev() {
        for hit in &entry.prs {
            let key = hit.pr.key();
            if dismissed.contains(&key) {
                continue;
            }
            match pr_at.get(&key) {
                Some(&at) => {
                    if hit.created {
                        prs[at].source = "created".into();
                    }
                }
                None => {
                    pr_at.insert(key, prs.len());
                    prs.push(PullRequestItem {
                        repo: hit.pr.repo.clone(),
                        number: hit.pr.number,
                        url: hit.pr.url.clone(),
                        title: None,
                        state: None,
                        checks: None,
                        entry_id: entry.entry_id.clone(),
                        source: if hit.created { "created" } else { "mentioned" }.into(),
                    });
                }
            }
        }
        for hit in &entry.links {
            match link_at.get(&hit.url) {
                Some(&at) => {
                    links[at].count += 1;
                    if links[at].title.is_none() {
                        links[at].title = hit.title.clone();
                    }
                }
                None => {
                    let Some(host) = host_of(&hit.url) else {
                        continue;
                    };
                    if links.len() >= MAX_LINKS {
                        truncated.links = Some(true);
                        continue;
                    }
                    link_at.insert(hit.url.clone(), links.len());
                    links.push(LinkItem {
                        url: hit.url.clone(),
                        host,
                        title: hit.title.clone(),
                        entry_id: entry.entry_id.clone(),
                        count: 1,
                    });
                }
            }
        }
        for attachment in &entry.attachments {
            if let (Some(path), Some(inside)) = (&attachment.path, &inside)
                && path.starts_with(inside.as_str())
            {
                continue;
            }
            let key = match (&attachment.attachment_id, &attachment.path) {
                (Some(id), _) => format!("a:{id}"),
                (None, Some(path)) => format!("p:{path}"),
                (None, None) => format!("e:{}:{}", entry.entry_id, attachment.name),
            };
            if !attachment_seen.insert(key) {
                continue;
            }
            if attachments.len() >= MAX_ATTACHMENTS {
                truncated.attachments = Some(true);
                continue;
            }
            attachments.push(attachment.clone());
        }
        for image in &entry.images {
            if images.len() >= MAX_IMAGES {
                truncated.images = Some(true);
                break;
            }
            images.push(image.clone());
        }
    }
    (prs, links, attachments, images, truncated)
}

// ── reading the transcript ──

fn messages_of(response: &Value) -> Vec<Value> {
    response
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

async fn newest_entry_id(
    session: &SessionClient,
    session_id: &str,
) -> Result<Option<String>, HarnessError> {
    let response = session
        .call_raw(
            "session::messages-tail",
            json!({ "session_id": session_id, "limit": 1, "include_custom": true,
                    "include_image_data": false }),
        )
        .await?;
    Ok(messages_of(&response)
        .last()
        .and_then(|item| item.get("entry_id"))
        .and_then(Value::as_str)
        .map(str::to_string))
}

async fn first_entry_id(
    session: &SessionClient,
    session_id: &str,
) -> Result<Option<String>, HarnessError> {
    let response = session
        .call_raw(
            "session::messages",
            json!({ "session_id": session_id, "limit": 1, "include_custom": true,
                    "include_image_data": false }),
        )
        .await?;
    Ok(messages_of(&response)
        .first()
        .and_then(|item| item.get("entry_id"))
        .and_then(Value::as_str)
        .map(str::to_string))
}

/// Every entry from `from` to `to` (both inclusive), page by page, into
/// `index`.
async fn scan_range(
    session: &SessionClient,
    session_id: &str,
    from: &str,
    to: &str,
    index: &mut SessionIndex,
    rescan_last: bool,
) -> Result<(), HarnessError> {
    let mut cursor: Option<String> = None;
    let mut first_page = true;
    loop {
        let mut payload = json!({
            "session_id": session_id, "from_entry_id": from, "to_entry_id": to,
            "limit": PAGE, "include_image_data": false,
        });
        if let Some(cursor) = &cursor {
            payload["cursor"] = json!(cursor);
        }
        let response = session.call_raw("session::messages-range", payload).await?;
        index.apply(&messages_of(&response), rescan_last && first_page);
        first_page = false;
        match response.get("next_cursor").and_then(Value::as_str) {
            Some(next) if !next.is_empty() && cursor.as_deref() != Some(next) => {
                cursor = Some(next.to_string());
            }
            _ => return Ok(()),
        }
    }
}

/// Bring `index` up to the session as `summary` reports it: nothing when
/// `message_count` and `updated_at` are unchanged, the new entries (and the
/// last one again) when it only grew, everything when its path moved.
async fn refresh(
    session: &SessionClient,
    summary: &SessionSummary,
    mut index: SessionIndex,
) -> Result<SessionIndex, HarnessError> {
    if index.scanned
        && index.message_count == summary.message_count
        && index.updated_at == summary.updated_at
    {
        return Ok(index);
    }
    let id = summary.session_id.as_str();
    let Some(newest) = newest_entry_id(session, id).await? else {
        return Ok(SessionIndex {
            message_count: summary.message_count,
            updated_at: summary.updated_at,
            scanned: true,
            ..SessionIndex::default()
        });
    };
    if let Some(last) = index.last_entry_id.clone() {
        let mut incremental = index.clone();
        match scan_range(session, id, &last, &newest, &mut incremental, true).await {
            Ok(()) => {
                incremental.message_count = summary.message_count;
                incremental.updated_at = summary.updated_at;
                return Ok(incremental);
            }
            Err(error) => {
                tracing::debug!(session_id = id, %error, "context index rescans from the start");
            }
        }
    }
    index = SessionIndex::default();
    if let Some(first) = first_entry_id(session, id).await? {
        scan_range(session, id, &first, &newest, &mut index, false).await?;
    }
    index.scanned = true;
    index.message_count = summary.message_count;
    index.updated_at = summary.updated_at;
    Ok(index)
}

// ── subagents ──

/// `running`, `done` or `failed`, from the session store's status.
pub fn subagent_status(status: Option<&str>) -> &'static str {
    match status {
        Some("working") => "running",
        Some("error") => "failed",
        _ => "done",
    }
}

fn subagent_item(child: &SessionSummary, descendants: u64) -> SubagentItem {
    let display = child
        .metadata
        .as_ref()
        .and_then(|m| m.get("subagent_display"))
        .and_then(|d| d.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let status = subagent_status(child.status.as_deref());
    SubagentItem {
        session_id: child.session_id.clone(),
        title: if child.title.trim().is_empty() {
            display.clone().unwrap_or_else(|| "Sub-agent".into())
        } else {
            child.title.clone()
        },
        status: status.into(),
        display,
        started_at_ms: child.created_at,
        ended_at_ms: (status != "running").then_some(child.updated_at),
        descendants,
    }
}

async fn children(
    session: &SessionClient,
    parent: &str,
) -> Result<Vec<SessionSummary>, HarnessError> {
    Ok(session
        .list_summaries(Some(json!({ "parent_session_id": parent })))
        .await?
        .into_iter()
        .filter(|child| child.meta_str("parent_session_id") == Some(parent))
        .collect())
}

async fn subagents(
    session: &SessionClient,
    session_id: &str,
) -> Result<Vec<SubagentItem>, HarnessError> {
    let mut direct = children(session, session_id).await?;
    direct.sort_by_key(|child| std::cmp::Reverse(child.created_at));
    let mut out = Vec::with_capacity(direct.len());
    for child in &direct {
        let mut count = 0u64;
        let mut queue = VecDeque::from([child.session_id.clone()]);
        let mut seen = HashSet::new();
        while let Some(id) = queue.pop_front() {
            if !seen.insert(id.clone()) || seen.len() > DESCENDANTS_CAP {
                continue;
            }
            for grandchild in children(session, &id).await? {
                count += 1;
                queue.push_back(grandchild.session_id);
            }
        }
        out.push(subagent_item(child, count));
    }
    Ok(out)
}

// ── side chats ──

pub fn side_chat_item(chat: &SessionSummary) -> SideChatItem {
    let status = subagent_status(chat.status.as_deref());
    SideChatItem {
        session_id: chat.session_id.clone(),
        title: if chat.title.trim().is_empty() {
            crate::side_chat::TITLE.into()
        } else {
            chat.title.clone()
        },
        message_count: crate::side_chat::own_message_count(chat),
        created_at_ms: chat.created_at,
        updated_at_ms: chat.updated_at,
        from_entry_id: crate::side_chat::fork_entry_id(chat),
        status: status.into(),
        running: status == "running",
    }
}

/// The conversation's side chats, most recently active first.
async fn side_chats(
    session: &SessionClient,
    session_id: &str,
) -> Result<Vec<SideChatItem>, HarnessError> {
    let mut chats = crate::side_chat::side_chats_of(session, session_id).await?;
    chats.sort_by_key(|chat| std::cmp::Reverse((chat.updated_at, chat.created_at)));
    Ok(chats.iter().map(side_chat_item).collect())
}

// ── pull request state ──

#[derive(Debug, Clone, PartialEq)]
struct PrReading {
    title: Option<String>,
    state: Option<String>,
    checks: Option<String>,
}

/// When a PR was read, and what was read (`None`: the read failed).
type PrCacheEntry = (Instant, Option<PrReading>);

/// Readings by canonical PR URL, each fresh for [`PR_TTL`].
static PR_CACHE: LazyLock<Mutex<HashMap<String, PrCacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// `open`, `draft`, `merged` or `closed` from `gh pr view`'s `state`/`isDraft`.
fn pr_state(state: &str, draft: bool) -> &'static str {
    match state.to_ascii_uppercase().as_str() {
        "MERGED" => "merged",
        "CLOSED" => "closed",
        _ if draft => "draft",
        _ => "open",
    }
}

/// One word for a PR's checks from `statusCheckRollup` (check runs carry
/// `status`/`conclusion`, commit statuses `state`): the worst wins.
pub fn summarize_checks(rollup: &[Value]) -> Option<&'static str> {
    if rollup.is_empty() {
        return None;
    }
    let upper = |v: &Value, key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_uppercase()
    };
    let mut pending = false;
    for check in rollup {
        let conclusion = upper(check, "conclusion");
        let status = upper(check, "status");
        let state = upper(check, "state");
        if matches!(
            conclusion.as_str(),
            "FAILURE" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE"
        ) || matches!(state.as_str(), "FAILURE" | "ERROR")
        {
            return Some("failing");
        }
        let check_run_open = !status.is_empty() && status != "COMPLETED";
        if check_run_open || matches!(state.as_str(), "PENDING" | "EXPECTED") {
            pending = true;
        }
    }
    Some(if pending { "pending" } else { "passing" })
}

fn parse_pr_view(stdout: &str) -> Option<PrReading> {
    let view: Value = serde_json::from_str(stdout.trim()).ok()?;
    let state = view.get("state").and_then(Value::as_str)?;
    let draft = view
        .get("isDraft")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let rollup = view
        .get("statusCheckRollup")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Some(PrReading {
        title: view
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string),
        state: Some(pr_state(state, draft).to_string()),
        checks: summarize_checks(&rollup).map(str::to_string),
    })
}

async fn read_pr(iii: Arc<iii_sdk::IIIClient>, url: String) {
    let request = iii_sdk::protocol::CallRequest {
        function_id: "github::exec".into(),
        payload: json!({
            "args": ["pr", "view", url, "--json", "number,title,state,isDraft,statusCheckRollup"],
            "timeout_ms": 20_000,
        }),
        action: None,
        timeout_ms: Some(25_000),
    };
    let reading = match iii.call(request).await {
        Ok(out) => {
            let out = out.get("value").cloned().unwrap_or(out);
            out.get("stdout")
                .and_then(Value::as_str)
                .and_then(parse_pr_view)
        }
        Err(error) => {
            tracing::debug!(%url, %error, "could not read a pull request's state");
            None
        }
    };
    PR_CACHE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(url, (Instant::now(), reading));
}

/// Fill each PR's state from the cache, reading the stale ones through
/// `github::exec` within [`PR_BUDGET`]; a reading that comes in later fills
/// the cache for the next call.
async fn read_pr_states(deps: &Deps, prs: &mut [PullRequestItem]) {
    if prs.is_empty() || deps.function_known_absent("github::exec").await {
        return;
    }
    let stale: Vec<String> = {
        let cache = PR_CACHE.lock().unwrap_or_else(|p| p.into_inner());
        prs.iter()
            .take(PR_READS)
            .filter(|pr| {
                cache
                    .get(&pr.url)
                    .is_none_or(|(at, _)| at.elapsed() > PR_TTL)
            })
            .map(|pr| pr.url.clone())
            .collect()
    };
    let tasks: Vec<_> = stale
        .into_iter()
        .map(|url| tokio::spawn(read_pr(deps.iii.clone(), url)))
        .collect();
    let _ = tokio::time::timeout(PR_BUDGET, futures_util::future::join_all(tasks)).await;
    let cache = PR_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    for pr in prs.iter_mut() {
        if let Some((_, Some(reading))) = cache.get(&pr.url) {
            pr.title = reading.title.clone();
            pr.state = reading.state.clone();
            pr.checks = reading.checks.clone();
        }
    }
}

// ── the function ──

pub async fn handle(
    deps: &Deps,
    request: ContextItemsRequest,
) -> Result<ContextItemsResponse, HarnessError> {
    let session = deps.session().await;
    let id = request.session_id.as_str();
    let summary = session
        .summary(id)
        .await?
        .ok_or_else(|| HarnessError::InvalidRequest(format!("session {id} does not exist")))?;
    let index = refresh(&session, &summary, deps.context_items.take(id)).await?;
    let (mut pull_requests, links, attachments, images, truncated) =
        aggregate(&index, summary.metadata.as_ref());
    deps.context_items.put(id, index);
    let subagents = subagents(&session, id).await.unwrap_or_else(|error| {
        tracing::warn!(session_id = id, %error, "could not list the subagents");
        Vec::new()
    });
    let side_chats = side_chats(&session, id).await.unwrap_or_else(|error| {
        tracing::warn!(session_id = id, %error, "could not list the side chats");
        Vec::new()
    });
    read_pr_states(deps, &mut pull_requests).await;
    Ok(ContextItemsResponse {
        session_id: summary.session_id.clone(),
        message_count: summary.message_count,
        generated_at_ms: crate::types::message::AgentMessage::now_ms(),
        subagents,
        pull_requests,
        links,
        attachments,
        images,
        side_chats,
        truncated,
    })
}

#[cfg(test)]
mod tests;
