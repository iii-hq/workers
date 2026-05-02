//! Session JSONL redaction and dataset publishing pipeline.
//!
//! Pipeline stages:
//! 1. [`scan_secrets`] — discovers secrets via TruffleHog (when on PATH) or a
//!    built-in regex catalog as a fallback.
//! 2. [`redact`] — replaces matched bytes with stable `[REDACTED:<kind>]`
//!    markers, then applies user-configured deny patterns.
//! 3. [`review`] — optional LLM safety pass over the redacted content.
//! 4. [`publish`] — POSTs the cleaned JSONL to a dataset platform endpoint.
//!
//! Workspace state (which sessions have already been processed) is tracked by
//! [`workspace_status`] reading `<workspace_dir>/state.json`.

use std::path::Path;
use std::process::Stdio;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

mod regexes {
    pub const AWS_ACCESS_KEY: &str = r"AKIA[0-9A-Z]{16}";
    pub const OPENAI: &str = r"sk-[A-Za-z0-9\-_]{20,}";
    pub const ANTHROPIC: &str = r"sk-ant-[A-Za-z0-9\-_]{20,}";
    pub const GITHUB_PAT: &str = r"ghp_[A-Za-z0-9]{36}";
    pub const GITHUB_FINEGRAINED: &str = r"github_pat_[A-Za-z0-9_]{82}";
    pub const STRIPE: &str = r"sk_live_[A-Za-z0-9]{24,}";
    pub const SLACK_BOT: &str = r"xoxb-[A-Za-z0-9-]+";
    pub const GOOGLE_API: &str = r"AIza[0-9A-Za-z\-_]{35}";
    pub const JWT: &str = r"eyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+";
    pub const RSA_BLOCK: &str = r"-----BEGIN (RSA|OPENSSH|EC) PRIVATE KEY-----";
    pub const AWS_SECRET_CANDIDATE: &str = r"[A-Za-z0-9/+=]{40}";
}

struct CompiledPattern {
    kind: &'static str,
    re: Regex,
}

static BUILTIN_PATTERNS: Lazy<Vec<CompiledPattern>> = Lazy::new(|| {
    vec![
        CompiledPattern {
            kind: "AWS Access Key",
            re: Regex::new(regexes::AWS_ACCESS_KEY).expect("aws access key regex"),
        },
        CompiledPattern {
            kind: "Anthropic API Key",
            re: Regex::new(regexes::ANTHROPIC).expect("anthropic regex"),
        },
        CompiledPattern {
            kind: "OpenAI API Key",
            re: Regex::new(regexes::OPENAI).expect("openai regex"),
        },
        CompiledPattern {
            kind: "GitHub Fine-grained PAT",
            re: Regex::new(regexes::GITHUB_FINEGRAINED).expect("github fine-grained regex"),
        },
        CompiledPattern {
            kind: "GitHub Classic PAT",
            re: Regex::new(regexes::GITHUB_PAT).expect("github pat regex"),
        },
        CompiledPattern {
            kind: "Stripe Secret Key",
            re: Regex::new(regexes::STRIPE).expect("stripe regex"),
        },
        CompiledPattern {
            kind: "Slack Bot Token",
            re: Regex::new(regexes::SLACK_BOT).expect("slack regex"),
        },
        CompiledPattern {
            kind: "Google API Key",
            re: Regex::new(regexes::GOOGLE_API).expect("google api regex"),
        },
        CompiledPattern {
            kind: "JWT",
            re: Regex::new(regexes::JWT).expect("jwt regex"),
        },
        CompiledPattern {
            kind: "Private Key Block",
            re: Regex::new(regexes::RSA_BLOCK).expect("rsa regex"),
        },
    ]
});

static AWS_SECRET_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(regexes::AWS_SECRET_CANDIDATE).expect("aws secret regex"));
static AWS_AKIA_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(regexes::AWS_ACCESS_KEY).expect("aws akia regex"));

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretMatch {
    pub kind: String,
    pub raw: String,
    pub line: usize,
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanReport {
    pub matches: Vec<SecretMatch>,
    pub trufflehog_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DenyPattern {
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewResult {
    pub safe_to_publish: bool,
    pub findings: Vec<String>,
    pub model: String,
}

#[async_trait]
pub trait ReviewLlm: Send + Sync {
    async fn review(&self, redacted_jsonl: &str, prompt: &str)
        -> Result<ReviewResult, CorpusError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishTarget {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    pub dataset_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceStatus {
    pub workspace_dir: String,
    pub processed_session_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<i64>,
    pub pending_count: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum CorpusError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("trufflehog binary not found on PATH")]
    TruffleHogMissing,
    #[error("subprocess error: {0}")]
    Subprocess(String),
    #[error("regex error: {0}")]
    Regex(#[from] regex::Error),
    #[error("http error: {0}")]
    Http(String),
    #[error("decode error: {0}")]
    Decode(String),
}

const DEFAULT_REVIEW_PROMPT: &str =
    "You are a privacy reviewer. Inspect the following session JSONL for any \
remaining sensitive content (API keys, tokens, personal addresses, customer \
data). Reply with safe_to_publish=true only if nothing remains. List concerns \
in `findings`.";

/// Scan session JSONL bytes for secrets.
pub async fn scan_secrets(session_jsonl: &str) -> Result<ScanReport, CorpusError> {
    if trufflehog_on_path().await {
        if let Ok(matches) = run_trufflehog(session_jsonl).await {
            return Ok(ScanReport {
                matches,
                trufflehog_available: true,
            });
        }
    }
    Ok(ScanReport {
        matches: builtin_scan(session_jsonl),
        trufflehog_available: false,
    })
}

async fn trufflehog_on_path() -> bool {
    Command::new("trufflehog")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn run_trufflehog(session_jsonl: &str) -> Result<Vec<SecretMatch>, CorpusError> {
    let mut child = Command::new("trufflehog")
        .arg("filesystem")
        .arg("--no-update")
        .arg("--json")
        .arg("/dev/stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| CorpusError::Subprocess(e.to_string()))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(session_jsonl.as_bytes()).await?;
        stdin.shutdown().await?;
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|e| CorpusError::Subprocess(e.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    parse_trufflehog_output(&stdout, session_jsonl)
}

fn parse_trufflehog_output(
    stdout: &str,
    session_jsonl: &str,
) -> Result<Vec<SecretMatch>, CorpusError> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let raw = v
            .get("Raw")
            .and_then(|r| r.as_str())
            .unwrap_or_default()
            .to_string();
        if raw.is_empty() {
            continue;
        }
        let kind = v
            .get("DetectorName")
            .and_then(|r| r.as_str())
            .unwrap_or("Unknown")
            .to_string();
        let verified = v.get("Verified").and_then(|r| r.as_bool()).unwrap_or(false);
        let line_no = locate_line(session_jsonl, &raw);
        out.push(SecretMatch {
            kind,
            raw,
            line: line_no,
            verified,
        });
    }
    Ok(out)
}

fn builtin_scan(session_jsonl: &str) -> Vec<SecretMatch> {
    let mut matches: Vec<SecretMatch> = Vec::new();
    for (idx, line) in session_jsonl.lines().enumerate() {
        let line_no = idx + 1;
        for pattern in BUILTIN_PATTERNS.iter() {
            for m in pattern.re.find_iter(line) {
                let raw = m.as_str().to_string();
                if pattern.kind == "OpenAI API Key" && raw.starts_with("sk-ant-") {
                    continue;
                }
                if !already_recorded(&matches, &raw) {
                    matches.push(SecretMatch {
                        kind: pattern.kind.to_string(),
                        raw,
                        line: line_no,
                        verified: false,
                    });
                }
            }
        }
        if AWS_AKIA_RE.is_match(line) {
            for m in AWS_SECRET_RE.find_iter(line) {
                let raw = m.as_str().to_string();
                if AWS_AKIA_RE.is_match(&raw) {
                    continue;
                }
                if !already_recorded(&matches, &raw) {
                    matches.push(SecretMatch {
                        kind: "AWS Secret Key".to_string(),
                        raw,
                        line: line_no,
                        verified: false,
                    });
                }
            }
        }
    }
    matches
}

fn already_recorded(matches: &[SecretMatch], raw: &str) -> bool {
    matches.iter().any(|m| m.raw == raw)
}

fn locate_line(session_jsonl: &str, needle: &str) -> usize {
    session_jsonl
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains(needle))
        .map_or(0, |(i, _)| i + 1)
}

/// Apply secret matches and user-supplied deny patterns to the input.
pub fn redact(
    session_jsonl: &str,
    secrets: &[SecretMatch],
    deny_patterns: &[DenyPattern],
) -> Result<String, CorpusError> {
    let mut sorted: Vec<&SecretMatch> = secrets.iter().collect();
    sorted.sort_by(|a, b| b.raw.len().cmp(&a.raw.len()));

    let mut text = session_jsonl.to_string();
    for s in sorted {
        if s.raw.is_empty() {
            continue;
        }
        let marker = format!("[REDACTED:{}]", s.kind);
        text = text.replace(&s.raw, &marker);
    }
    for deny in deny_patterns {
        let re = Regex::new(&deny.pattern)?;
        let label = deny.description.as_deref().unwrap_or("DENY");
        let marker = format!("[REDACTED:{label}]");
        text = re.replace_all(&text, marker.as_str()).into_owned();
    }
    Ok(text)
}

/// Run the LLM safety review.
pub async fn review<R: ReviewLlm + ?Sized>(
    reviewer: &R,
    redacted_jsonl: &str,
    custom_prompt: Option<&str>,
) -> Result<ReviewResult, CorpusError> {
    let prompt = custom_prompt.unwrap_or(DEFAULT_REVIEW_PROMPT);
    reviewer.review(redacted_jsonl, prompt).await
}

/// Publish the cleaned JSONL to a dataset platform.
pub async fn publish(
    redacted_jsonl: &str,
    target: &PublishTarget,
    metadata: serde_json::Value,
) -> Result<String, CorpusError> {
    let mut hasher = Sha256::new();
    hasher.update(redacted_jsonl.as_bytes());
    let digest = hex::encode(hasher.finalize());

    let body = serde_json::json!({
        "dataset_id": target.dataset_id,
        "content": redacted_jsonl,
        "content_sha256": digest,
        "metadata": metadata,
    });
    let client = reqwest::Client::new();
    let mut req = client.post(&target.url).json(&body);
    if let Some(token) = &target.auth_token {
        req = req.bearer_auth(token);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| CorpusError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(CorpusError::Http(format!(
            "publish failed: status={}",
            resp.status()
        )));
    }
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(ToString::to_string);
    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CorpusError::Decode(e.to_string()))?;
    if let Some(url) = value.get("url").and_then(|v| v.as_str()) {
        return Ok(url.to_string());
    }
    location.ok_or_else(|| CorpusError::Decode("missing url in response".into()))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PersistedState {
    #[serde(default)]
    processed_session_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_run_at: Option<i64>,
    #[serde(default)]
    pending_count: u32,
}

/// Read `<workspace_dir>/state.json` describing what has been processed.
pub async fn workspace_status(workspace_dir: &Path) -> Result<WorkspaceStatus, CorpusError> {
    let state_path = workspace_dir.join("state.json");
    if !state_path.exists() {
        return Ok(WorkspaceStatus {
            workspace_dir: workspace_dir.display().to_string(),
            processed_session_ids: Vec::new(),
            last_run_at: None,
            pending_count: 0,
        });
    }
    let bytes = tokio::fs::read(&state_path).await?;
    let state: PersistedState = serde_json::from_slice(&bytes)
        .map_err(|e| CorpusError::Decode(format!("state.json: {e}")))?;
    Ok(WorkspaceStatus {
        workspace_dir: workspace_dir.display().to_string(),
        processed_session_ids: state.processed_session_ids,
        last_run_at: state.last_run_at,
        pending_count: state.pending_count,
    })
}

/// Registered function ids exposed by [`register_with_iii`].
pub mod function_ids {
    pub const SCAN: &str = "corpus::scan";
    pub const REDACT: &str = "corpus::redact";
    pub const REVIEW: &str = "corpus::review";
    pub const PUBLISH: &str = "corpus::publish";
}

/// Register the four `corpus::*` iii functions on `iii`.
///
/// The `review` function delegates to a [`ReviewLlm`] supplied by the caller.
/// Pass `None` to register a stub that returns `safe_to_publish=true` with an
/// empty findings list — useful when no LLM reviewer is wired yet.
///
/// # Payload shapes
///
/// - `corpus::scan` — `{ "session_jsonl": str }` → [`ScanReport`]
/// - `corpus::redact` — `{ "session_jsonl": str, "secrets": [SecretMatch],
///   "deny_patterns": [DenyPattern] }` → `{ "redacted": str }`
/// - `corpus::review` — `{ "redacted_jsonl": str, "custom_prompt": str? }`
///   → [`ReviewResult`]
/// - `corpus::publish` — `{ "redacted_jsonl": str, "target": PublishTarget,
///   "metadata": Value }` → `{ "url": str }`
pub fn register_with_iii(
    iii: &iii_sdk::III,
    reviewer: Option<std::sync::Arc<dyn ReviewLlm>>,
) -> CorpusFunctionRefs {
    use iii_sdk::{IIIError, RegisterFunctionMessage};
    use serde_json::json;

    let mut refs: Vec<iii_sdk::FunctionRef> = Vec::with_capacity(4);

    refs.push(iii.register_function((
        RegisterFunctionMessage::with_id(function_ids::SCAN.into()).with_description(
            "Scan session JSONL for secrets via TruffleHog or builtin regex".into(),
        ),
        move |payload: serde_json::Value| async move {
            let session_jsonl = required_str(&payload, "session_jsonl")?;
            let report = scan_secrets(&session_jsonl)
                .await
                .map_err(|e| IIIError::Handler(e.to_string()))?;
            serde_json::to_value(report).map_err(|e| IIIError::Handler(e.to_string()))
        },
    )));

    refs.push(iii.register_function((
        RegisterFunctionMessage::with_id(function_ids::REDACT.into()).with_description(
            "Apply secret matches and user deny patterns to session JSONL".into(),
        ),
        move |payload: serde_json::Value| async move {
            let session_jsonl = required_str(&payload, "session_jsonl")?;
            let secrets: Vec<SecretMatch> = payload
                .get("secrets")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| IIIError::Handler(e.to_string()))?
                .unwrap_or_default();
            let deny: Vec<DenyPattern> = payload
                .get("deny_patterns")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| IIIError::Handler(e.to_string()))?
                .unwrap_or_default();
            let redacted = redact(&session_jsonl, &secrets, &deny)
                .map_err(|e| IIIError::Handler(e.to_string()))?;
            Ok(json!({ "redacted": redacted }))
        },
    )));

    let reviewer_for_handler = reviewer;
    refs.push(
        iii.register_function((
            RegisterFunctionMessage::with_id(function_ids::REVIEW.into())
                .with_description("Run an LLM safety review over redacted JSONL".into()),
            move |payload: serde_json::Value| {
                let reviewer = reviewer_for_handler.clone();
                async move {
                    let redacted_jsonl = required_str(&payload, "redacted_jsonl")?;
                    let custom_prompt = payload
                        .get("custom_prompt")
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string);
                    let result = match reviewer {
                        Some(r) => review(r.as_ref(), &redacted_jsonl, custom_prompt.as_deref())
                            .await
                            .map_err(|e| IIIError::Handler(e.to_string()))?,
                        None => ReviewResult {
                            safe_to_publish: true,
                            findings: Vec::new(),
                            model: "noop".into(),
                        },
                    };
                    serde_json::to_value(result).map_err(|e| IIIError::Handler(e.to_string()))
                }
            },
        )),
    );

    refs.push(
        iii.register_function((
            RegisterFunctionMessage::with_id(function_ids::PUBLISH.into())
                .with_description("Upload redacted JSONL to a dataset platform".into()),
            move |payload: serde_json::Value| async move {
                let redacted_jsonl = required_str(&payload, "redacted_jsonl")?;
                let target: PublishTarget = payload
                    .get("target")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|e| IIIError::Handler(e.to_string()))?
                    .ok_or_else(|| IIIError::Handler("missing required field: target".into()))?;
                let metadata = payload
                    .get("metadata")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let url = publish(&redacted_jsonl, &target, metadata)
                    .await
                    .map_err(|e| IIIError::Handler(e.to_string()))?;
                Ok(json!({ "url": url }))
            },
        )),
    );

    CorpusFunctionRefs { refs }
}

/// Handle returned by [`register_with_iii`].
pub struct CorpusFunctionRefs {
    refs: Vec<iii_sdk::FunctionRef>,
}

impl CorpusFunctionRefs {
    pub fn unregister_all(self) {
        for f in self.refs {
            f.unregister();
        }
    }

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
    use std::sync::atomic::{AtomicU64, Ordering};

    fn sm(kind: &str, raw: &str, line: usize) -> SecretMatch {
        SecretMatch {
            kind: kind.into(),
            raw: raw.into(),
            line,
            verified: false,
        }
    }

    fn unique_dir() -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let mut p = std::env::temp_dir();
        let suffix = format!(
            "session-corpus-{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        p.push(suffix);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn cleanup(p: &std::path::Path) {
        let _ = std::fs::remove_dir_all(p);
    }

    #[test]
    fn redact_replaces_with_marker() {
        let raw = format!("sk-{}", "z".repeat(28));
        let input = format!("{{\"key\":\"{raw}\"}}");
        let secrets = vec![sm("OpenAI API Key", &raw, 1)];
        let out = redact(&input, &secrets, &[]).unwrap();
        assert!(out.contains("[REDACTED:OpenAI API Key]"));
        assert!(!out.contains(&raw));
    }

    #[test]
    fn redact_is_noop_without_secrets() {
        let input = "{\"hello\":\"world\"}";
        let out = redact(input, &[], &[]).unwrap();
        assert_eq!(out, input);
    }

    #[test]
    fn deny_patterns_apply_after_secrets() {
        let raw = format!("sk-{}", "z".repeat(28));
        let input = format!("user@example.com and {raw}");
        let secrets = vec![sm("OpenAI API Key", &raw, 1)];
        let deny = vec![DenyPattern {
            pattern: r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}".into(),
            description: Some("EMAIL".into()),
        }];
        let out = redact(&input, &secrets, &deny).unwrap();
        assert!(out.contains("[REDACTED:OpenAI API Key]"));
        assert!(out.contains("[REDACTED:EMAIL]"));
        assert!(!out.contains("user@example.com"));
    }

    #[test]
    fn builtin_scan_finds_each_pattern_shape() {
        let aws = format!("AKIA{}", "X".repeat(16));
        let openai = format!("sk-{}", "0".repeat(40));
        let anthropic = format!("sk-ant-{}", "0".repeat(40));
        let ghpat = format!("ghp_{}", "0".repeat(36));
        let stripe = format!("sk_live_{}", "0".repeat(24));
        let slack = "xoxb-1-2-aaa".to_string();
        let google = format!("AIza{}", "0".repeat(35));
        let jwt = format!(
            "{}.{}.{}",
            "eyJ0eXAiOiJ0ZXN0In0",
            "eyJzdWIiOiIxMjMifQ",
            "0".repeat(20)
        );
        let private_block = "-----BEGIN RSA PRIVATE KEY-----".to_string();

        let cases: [(&str, &str); 9] = [
            ("AWS Access Key", &aws),
            ("OpenAI API Key", &openai),
            ("Anthropic API Key", &anthropic),
            ("GitHub Classic PAT", &ghpat),
            ("Stripe Secret Key", &stripe),
            ("Slack Bot Token", &slack),
            ("Google API Key", &google),
            ("JWT", &jwt),
            ("Private Key Block", &private_block),
        ];
        for (kind, line) in &cases {
            let hits = builtin_scan(line);
            assert!(
                hits.iter().any(|h| h.kind == *kind),
                "expected to detect {kind} in {line:?}: got {hits:?}"
            );
        }
    }

    #[test]
    fn aws_secret_only_when_akia_present() {
        let akia = format!("AKIA{}", "X".repeat(16));
        let secret_chars: String = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string();
        let with_akia = format!("{akia} {secret_chars}");
        let hits = builtin_scan(&with_akia);
        assert!(hits.iter().any(|h| h.kind == "AWS Access Key"));
        assert!(hits.iter().any(|h| h.kind == "AWS Secret Key"));

        let without = format!("{secret_chars} alone on a line");
        let hits = builtin_scan(&without);
        assert!(!hits.iter().any(|h| h.kind == "AWS Secret Key"));
    }

    #[tokio::test]
    async fn scan_secrets_uses_builtin_when_trufflehog_missing() {
        if trufflehog_on_path().await {
            return;
        }
        let raw = format!("sk-{}", "0".repeat(40));
        let report = scan_secrets(&raw).await.unwrap();
        assert!(!report.trufflehog_available);
        assert!(report.matches.iter().any(|m| m.kind == "OpenAI API Key"));
    }

    #[tokio::test]
    async fn workspace_status_empty_when_no_state() {
        let dir = unique_dir();
        let status = workspace_status(&dir).await.unwrap();
        assert!(status.processed_session_ids.is_empty());
        assert_eq!(status.pending_count, 0);
        assert!(status.last_run_at.is_none());
        cleanup(&dir);
    }

    #[tokio::test]
    async fn workspace_status_reads_state_json() {
        let dir = unique_dir();
        let state_path = dir.join("state.json");
        let body = serde_json::json!({
            "processed_session_ids": ["a", "b"],
            "last_run_at": 1_700_000_000_i64,
            "pending_count": 3
        });
        tokio::fs::write(&state_path, body.to_string())
            .await
            .unwrap();
        let status = workspace_status(&dir).await.unwrap();
        assert_eq!(status.processed_session_ids, vec!["a", "b"]);
        assert_eq!(status.last_run_at, Some(1_700_000_000));
        assert_eq!(status.pending_count, 3);
        cleanup(&dir);
    }

    #[test]
    fn review_result_roundtrip() {
        let r = ReviewResult {
            safe_to_publish: false,
            findings: vec!["leaked email".into()],
            model: "test-model".into(),
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: ReviewResult = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }

    struct MockReviewer {
        safe: bool,
    }

    #[async_trait]
    impl ReviewLlm for MockReviewer {
        async fn review(
            &self,
            _redacted: &str,
            _prompt: &str,
        ) -> Result<ReviewResult, CorpusError> {
            Ok(ReviewResult {
                safe_to_publish: self.safe,
                findings: if self.safe {
                    Vec::new()
                } else {
                    vec!["found something".into()]
                },
                model: "mock".into(),
            })
        }
    }

    #[tokio::test]
    async fn review_uses_default_prompt() {
        let mock = MockReviewer { safe: true };
        let result = review(&mock, "{}", None).await.unwrap();
        assert!(result.safe_to_publish);
        assert_eq!(result.model, "mock");
    }

    #[tokio::test]
    async fn review_threads_unsafe_findings() {
        let mock = MockReviewer { safe: false };
        let result = review(&mock, "{}", Some("strict")).await.unwrap();
        assert!(!result.safe_to_publish);
        assert_eq!(result.findings.len(), 1);
    }
}
