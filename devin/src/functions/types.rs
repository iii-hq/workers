//! Request payloads. Each derives `JsonSchema` so the engine publishes the
//! parameter table for `iii trigger <fn> --help`, and `Deserialize` so the
//! handler parses at the unknown boundary.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Message {
    pub role: String,
    /// Either a plain string or an array of content blocks.
    pub content: Value,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RunRequest {
    /// iii session id; reuse to keep the same local record. Omit to generate one.
    pub session_id: Option<String>,
    /// The prompt handed to the `devin` CLI for this turn.
    pub prompt: Option<String>,
    /// Alternative to prompt: role/content messages; the last user entry becomes the prompt.
    pub messages: Option<Vec<Message>>,
    /// Working directory the CLI runs in. Empty = the worker's process directory.
    pub cwd: Option<String>,
    /// Prepend the iii runtime discovery prompt as leading instructions (default from config).
    pub iii_context: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SessionIdRequest {
    /// iii session id (devin::run) or Devin cloud session id (devin::session::*).
    pub session_id: String,
}

/// A raw pass-through call to any Devin v3 endpoint. Use this for anything the
/// typed wrappers do not cover. Paths are relative to the configured base URL
/// (e.g. `organizations/{org_id}/sessions`).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ApiRequest {
    /// HTTP method: GET, POST, PUT, PATCH, or DELETE.
    pub method: String,
    /// Path relative to the configured base URL, e.g. `organizations/{org_id}/sessions`.
    pub path: String,
    /// Optional query parameters as a flat object.
    pub query: Option<Value>,
    /// Optional JSON request body.
    pub body: Option<Value>,
}

/// Create a Devin cloud session. `prompt` (or a user `messages` entry) is
/// required; every other field maps to a documented `POST /organizations/{org_id}/sessions`
/// field and is omitted from the body when absent.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SessionCreateRequest {
    /// The task prompt for the new Devin session.
    pub prompt: Option<String>,
    /// Alternative to prompt: role/content messages; the last user entry becomes the prompt.
    pub messages: Option<Vec<Message>>,
    /// Human-readable session title.
    pub title: Option<String>,
    /// Agent mode: normal, fast, lite, ultra, or fusion.
    pub devin_mode: Option<String>,
    /// Repository identifiers to attach to the session.
    pub repos: Option<Vec<String>>,
    /// File attachment URLs.
    pub attachment_urls: Option<Vec<String>>,
    /// Playbook id to run.
    pub playbook_id: Option<String>,
    /// Knowledge ids to attach to the session.
    pub knowledge_ids: Option<Vec<String>>,
    /// Secret ids to expose to the session.
    pub secret_ids: Option<Vec<String>>,
    /// Cap the session's ACU consumption.
    pub max_acu_limit: Option<u64>,
    /// Preserve VM state so the session can be resumed (default true).
    pub resumable: Option<bool>,
    /// Skip approval workflows for this session.
    pub bypass_approval: Option<bool>,
    /// Tags to apply to the session.
    pub tags: Option<Vec<String>>,
}

impl SessionCreateRequest {
    /// Build the `POST /organizations/{org_id}/sessions` body, inserting
    /// `prompt` and only the fields the caller supplied.
    pub fn to_body(&self, prompt: &str) -> Value {
        let mut m = Map::new();
        m.insert("prompt".into(), json!(prompt));
        if let Some(v) = &self.title {
            m.insert("title".into(), json!(v));
        }
        if let Some(v) = &self.devin_mode {
            m.insert("devin_mode".into(), json!(v));
        }
        if let Some(v) = &self.repos {
            m.insert("repos".into(), json!(v));
        }
        if let Some(v) = &self.attachment_urls {
            m.insert("attachment_urls".into(), json!(v));
        }
        if let Some(v) = &self.playbook_id {
            m.insert("playbook_id".into(), json!(v));
        }
        if let Some(v) = &self.knowledge_ids {
            m.insert("knowledge_ids".into(), json!(v));
        }
        if let Some(v) = &self.secret_ids {
            m.insert("secret_ids".into(), json!(v));
        }
        if let Some(v) = self.max_acu_limit {
            m.insert("max_acu_limit".into(), json!(v));
        }
        if let Some(v) = self.resumable {
            m.insert("resumable".into(), json!(v));
        }
        if let Some(v) = self.bypass_approval {
            m.insert("bypass_approval".into(), json!(v));
        }
        if let Some(v) = &self.tags {
            m.insert("tags".into(), json!(v));
        }
        Value::Object(m)
    }
}

/// List Devin cloud sessions.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SessionListRequest {
    /// Maximum number of sessions to return.
    pub limit: Option<u64>,
    /// Pagination offset.
    pub offset: Option<u64>,
    /// Filter by tags.
    pub tags: Option<Vec<String>>,
}

impl SessionListRequest {
    pub fn to_query(&self) -> Value {
        let mut m = Map::new();
        if let Some(v) = self.limit {
            m.insert("limit".into(), json!(v));
        }
        if let Some(v) = self.offset {
            m.insert("offset".into(), json!(v));
        }
        if let Some(v) = &self.tags {
            if !v.is_empty() {
                m.insert("tags".into(), json!(v.join(",")));
            }
        }
        Value::Object(m)
    }
}

/// Send a follow-up message to a running Devin session.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SessionMessageRequest {
    /// Devin cloud session id (returned by devin::session::create).
    pub session_id: String,
    /// The message to send.
    pub message: String,
}

/// Trigger a Devin PR review for a pull/merge request.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PrReviewTriggerRequest {
    /// Full URL of the pull/merge request to review.
    pub pr_url: String,
}

/// Look up the latest Devin PR review for a pull/merge request.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PrReviewStatusRequest {
    /// Full URL of the pull/merge request to look up.
    pub pr_url: String,
    /// Commit SHA (full or short); defaults to the PR head when omitted.
    pub commit_sha: Option<String>,
}

impl PrReviewStatusRequest {
    pub fn to_query(&self) -> Value {
        let mut m = Map::new();
        m.insert("pr_url".into(), json!(self.pr_url));
        if let Some(v) = &self.commit_sha {
            m.insert("commit_sha".into(), json!(v));
        }
        Value::Object(m)
    }
}

/// List enterprise code scan findings (enterprise-gated).
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CodeScanFindingsRequest {
    /// Pagination cursor.
    pub after: Option<String>,
    /// Page size (1-200, default 100).
    pub first: Option<u64>,
    /// Filter by organization ids.
    pub org_ids: Option<Vec<String>>,
    /// Filter by scan id.
    pub scan_id: Option<String>,
    /// Filter by repository name.
    pub repo_name: Option<String>,
    /// Filter by severity: critical, high, medium, low.
    pub severity: Option<Vec<String>>,
    /// Filter by status: open, dismissed, resolved.
    pub status: Option<Vec<String>>,
}

impl CodeScanFindingsRequest {
    pub fn to_query(&self) -> Value {
        let mut m = Map::new();
        if let Some(v) = &self.after {
            m.insert("after".into(), json!(v));
        }
        if let Some(v) = self.first {
            m.insert("first".into(), json!(v));
        }
        if let Some(v) = &self.org_ids {
            if !v.is_empty() {
                m.insert("org_ids".into(), json!(v.join(",")));
            }
        }
        if let Some(v) = &self.scan_id {
            m.insert("scan_id".into(), json!(v));
        }
        if let Some(v) = &self.repo_name {
            m.insert("repo_name".into(), json!(v));
        }
        if let Some(v) = &self.severity {
            if !v.is_empty() {
                m.insert("severity".into(), json!(v.join(",")));
            }
        }
        if let Some(v) = &self.status {
            if !v.is_empty() {
                m.insert("status".into(), json!(v.join(",")));
            }
        }
        Value::Object(m)
    }
}

/// Get metrics for enterprise code scans (enterprise-gated).
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CodeScanMetricsRequest {
    /// Filter by scan id.
    pub scan_id: Option<String>,
    /// Filter by repository name.
    pub repo_name: Option<String>,
    /// Filter by organization ids.
    pub org_ids: Option<Vec<String>>,
}

impl CodeScanMetricsRequest {
    pub fn to_query(&self) -> Value {
        let mut m = Map::new();
        if let Some(v) = &self.scan_id {
            m.insert("scan_id".into(), json!(v));
        }
        if let Some(v) = &self.repo_name {
            m.insert("repo_name".into(), json!(v));
        }
        if let Some(v) = &self.org_ids {
            if !v.is_empty() {
                m.insert("org_ids".into(), json!(v.join(",")));
            }
        }
        Value::Object(m)
    }
}

/// Launch a remediation session for a code scan finding (enterprise-gated).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CodeScanRemediateRequest {
    /// The scan the finding belongs to.
    pub scan_id: String,
    /// The finding to remediate.
    pub finding_id: String,
}

/// Extract the prompt from a run-style payload: explicit `prompt` (incl. empty
/// string) wins; otherwise the last user message's text.
fn extract_from(
    prompt: &Option<String>,
    messages: &Option<Vec<Message>>,
) -> anyhow::Result<String> {
    if let Some(p) = prompt {
        return Ok(p.clone());
    }
    let messages = messages
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("requires `prompt` or a user message in `messages`"))?;
    let last = messages
        .iter()
        .rfind(|m| m.role == "user")
        .ok_or_else(|| anyhow::anyhow!("requires `prompt` or a user message in `messages`"))?;
    match &last.content {
        Value::String(s) => Ok(s.clone()),
        Value::Array(blocks) => {
            let text = blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                return Err(anyhow::anyhow!("message content has no text blocks"));
            }
            Ok(text)
        }
        _ => Err(anyhow::anyhow!(
            "unsupported message content: expected a string or an array of content blocks"
        )),
    }
}

pub fn extract_prompt(req: &RunRequest) -> anyhow::Result<String> {
    extract_from(&req.prompt, &req.messages)
}

pub fn extract_create_prompt(req: &SessionCreateRequest) -> anyhow::Result<String> {
    extract_from(&req.prompt, &req.messages)
}
