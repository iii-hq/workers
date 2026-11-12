//! Private, best-effort byte witnesses at the final SDK dispatch boundary.
//!
//! Not a Memory relevance certificate or proof of provider receipt. Consumers
//! must bind a successful terminal and its internal assistant_entry_id. The
//! assistant update event's origin.generation_id is a correlation hint ONLY:
//! require an independently authenticated Harness update, never trust origin.
//! then match full fact lines from independently observed legitimate Memory
//! delivery, including allowed role/location and provenance. A paraphrase is
//! NOT_MEASURED. SHA-256 is not anonymization: finite-source dictionary attacks
//! remain possible. Engine access policy must restrict observers accordingly;
//! `private` authenticates the emitting connection, not observer confidentiality.
//! No raw history/replay store, prompt endpoint, tools, writer refs or error text.
//! v2 strict consumers require complete AND origins_complete plus a matching
//! authenticated success terminal. Sources bind exact final JSON message slots;
//! absent sources never certify Memory, even for identical text. Receipts are
//! bounded process-local digests, not durable authentication across restarts.
//! Summary certificates bind renderer byte spans to actual authenticated input
//! receipts; only contiguous ordered exact source lines are measured. Legacy
//! unbound summaries remain incomplete; paraphrases are not measured.
//! Default OFF, including test builds. The operator may set process-start
//! III_HARNESS_SYNTHETIC_GENERATION_CAPTURE=1 ONLY in a separate isolated
//! synthetic stack. Any other value is denied. Read once at event registration,
//! never from turn options, hooks or metadata. UNSAFE outside that boundary:
//! the operator is responsible for isolating readers and excluding real data;
//! the SDK exposes observer target/args, not authenticated reader identity.
//! Sequence is diagnostic, not an all-global continuity fence: correlate each
//! request/success/current update and the required source receipt independently.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use iii_sdk::{CallAction, CallRequest, IIIClient};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Source authority is in-process only; no Deserialize or caller JSON path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppendSource {
    pub observer_id: String,
    pub append_ordinal: usize,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SourceHints {
    pub slots: Vec<SourceSlot>,
    pub summary: Option<SummaryWitness>,
    pub invalid: bool,
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SourceSlot {
    pub message_index: usize,
    pub entry_id: String,
    pub message_sha256: String,
    pub observer_id: String,
    pub append_ordinal: usize,
    /// fresh_memory or replay_memory; never inferred from content.
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SummarySpan {
    pub byte_start: usize,
    pub byte_end: usize,
    pub summary_sha256: String,
    pub system_sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct AssemblySummary {
    pub assembly_id: String,
    pub span: SummarySpan,
    pub summarized_indices: Option<Vec<usize>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SummaryWitness {
    pub assembly_id: String,
    pub span: SummarySpan,
    pub inputs: Vec<SourceSlot>,
    /// Only exact full lines of authenticated Memory inputs, in summary order.
    pub lines: Vec<LineWitness>,
    /// Line digests of each certified group, replayed whole. In-process only;
    /// a witness without them replays the contiguous runs of `lines`.
    #[serde(skip)]
    pub groups: Vec<Vec<String>>,
}

type SummaryLedger = std::collections::VecDeque<(String, String, SummaryWitness)>;
static SUMMARY_RECEIPTS: OnceLock<std::sync::Mutex<SummaryLedger>> = OnceLock::new();

pub(crate) fn raw_digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub(crate) fn valid_span<'a>(system: &'a str, span: &SummarySpan) -> Option<&'a str> {
    if system.len() > MAX_BYTES || raw_digest(system) != span.system_sha256 {
        return None;
    }
    let text = system.get(span.byte_start..span.byte_end)?;
    (span.byte_end == system.len() && raw_digest(text) == span.summary_sha256).then_some(text)
}

pub(crate) fn bind_summary(
    session: &str,
    origin: &AssemblySummary,
    system: &str,
    candidate: &[(String, crate::types::message::AgentMessage)],
    previous: Option<(&str, &str)>,
) -> Option<SummaryWitness> {
    let summary = valid_span(system, &origin.span)?;
    if origin.assembly_id.len() > 256 {
        return None;
    }
    let mut result = SummaryWitness {
        assembly_id: origin.assembly_id.clone(),
        span: origin.span.clone(),
        inputs: Vec::new(),
        lines: Vec::new(),
        groups: Vec::new(),
    };
    let mut allowed: Vec<Vec<String>> = Vec::new();
    if let Some(indices) = &origin.summarized_indices {
        if indices.len() > MAX_LINES {
            return None;
        }
        let mut seen = std::collections::HashSet::new();
        for input in indices {
            if !seen.insert(*input) {
                return None;
            }
            let (id, message) = candidate.get(*input)?;
            // Memory notices are user text only. Bound the typed source before
            // converting to Value; never serialize arbitrary tools/media here.
            let crate::types::message::AgentMessage::User(user) = message else {
                continue;
            };
            if user.content.len() > MAX_LINES {
                return None;
            }
            let mut bytes = 0usize;
            for block in &user.content {
                let crate::types::content::ContentBlock::Text { text } = block else {
                    return None;
                };
                bytes = bytes.saturating_add(text.len());
                if bytes > MAX_BYTES {
                    return None;
                }
            }
            let value = serde_json::to_value(message).ok()?;
            if let Some(slot) = replay_slot(session, id, &value, *input) {
                let mut capture = GenerationEvent::new("", "", 0);
                capture.content(&value["content"], "user", &mut vec![*input]);
                if !capture.complete {
                    return None;
                }
                let total: usize = allowed.iter().map(Vec::len).sum();
                if total.saturating_add(capture.lines.len()) > MAX_LINES {
                    return None;
                }
                allowed.push(capture.lines.into_iter().map(|line| line.sha256).collect());
                result.inputs.push(slot);
            }
        }
    }
    if let Some((entry, digest)) = previous {
        let prior = with_summary_ledger(|ledger| {
            ledger
                .iter()
                .find(|(s, e, w)| s == session && e == entry && w.span.summary_sha256 == digest)
                .map(|(_, _, w)| w.clone())
        })?;
        if let Some(prior) = prior {
            if result.inputs.len() + prior.inputs.len() > MAX_LINES {
                return None;
            }
            // Replay each certified group on its own: a later compaction may put
            // summary text between two groups that were once adjacent. A witness
            // without group boundaries falls back to its contiguous runs.
            let groups: Vec<Vec<String>> = if prior.groups.is_empty() {
                prior
                    .lines
                    .chunk_by(|a, b| b.line == a.line + 1)
                    .map(|run| run.iter().map(|line| line.sha256.clone()).collect())
                    .collect()
            } else {
                prior.groups
            };
            if allowed.iter().chain(&groups).map(Vec::len).sum::<usize>() > MAX_LINES {
                return None;
            }
            allowed.extend(groups);
            result.inputs.extend(prior.inputs);
        } else if origin.summarized_indices.is_none() {
            return None;
        }
    } else if origin.summarized_indices.is_none() {
        return None;
    }
    if result.inputs.is_empty() {
        return None;
    }
    let first_line = system[..origin.span.byte_start]
        .bytes()
        .filter(|b| *b == b'\n')
        .count();
    let summary_lines: Vec<_> = summary
        .lines()
        .take(MAX_LINES + 1)
        .enumerate()
        .filter_map(|(line, text)| line_digest(text).map(|digest| (line, digest)))
        .collect();
    if summary_lines.len() > MAX_LINES {
        return None;
    }
    // Exact ordered contiguous source sequence, within the renderer's span.
    // Never promote scattered/reordered matching user text into a certificate.
    for source in allowed {
        if source.is_empty() {
            continue;
        }
        if let Some(start) = summary_lines.windows(source.len()).position(|window| {
            window
                .iter()
                .zip(&source)
                .all(|((_, actual), expected)| actual == expected)
                && window.windows(2).all(|pair| pair[1].0 == pair[0].0 + 1)
        }) {
            for (line, digest) in &summary_lines[start..start + source.len()] {
                if result.lines.len() == MAX_LINES {
                    return None;
                }
                result.lines.push(LineWitness {
                    role: "system".into(),
                    location: vec![],
                    line: first_line + line,
                    sha256: digest.clone(),
                });
            }
            result.groups.push(source);
        }
    }
    if result.lines.is_empty() {
        return None;
    }
    result.lines.sort_by_key(|line| line.line);
    result.lines.dedup_by_key(|line| line.line);
    Some(result)
}

pub(crate) fn remember_summary(session: &str, entry: &str, witness: &SummaryWitness) {
    if session.len() > 256
        || entry.len() > 256
        || witness.inputs.len() > MAX_LINES
        || witness.lines.len() > MAX_LINES
    {
        return;
    }
    with_summary_ledger(|ledger| {
        ledger.retain(|(s, e, _)| s != session || e != entry);
        if ledger.len() == MAX_LINES {
            ledger.pop_front();
        }
        ledger.push_back((session.into(), entry.into(), witness.clone()));
    });
}

// Producer-owned receipts: bounded and never reconstructed from custom data.
// Restarts/eviction deliberately make old notices unbound, not self-authenticating.
type SlotLedger = std::collections::VecDeque<(String, SourceSlot)>;
static RECEIPTS: OnceLock<std::sync::Mutex<SlotLedger>> = OnceLock::new();

// Isolated test ledgers execute these exact production methods without
// clearing, filling or evicting any concurrently running test's global state.
#[cfg(test)]
thread_local! {
    static TEST_SLOT_LEDGER: std::cell::RefCell<Option<SlotLedger>> = const { std::cell::RefCell::new(None) };
    static TEST_SUMMARY_LEDGER: std::cell::RefCell<Option<SummaryLedger>> = const { std::cell::RefCell::new(None) };
}

fn with_slot_ledger<T>(f: impl FnOnce(&mut SlotLedger) -> T) -> Option<T> {
    #[cfg(test)]
    if TEST_SLOT_LEDGER.with(|l| l.borrow().is_some()) {
        return TEST_SLOT_LEDGER.with(|l| Some(f(l.borrow_mut().as_mut().unwrap())));
    }
    let mut ledger = RECEIPTS.get_or_init(Default::default).lock().ok()?;
    Some(f(&mut ledger))
}

fn with_summary_ledger<T>(f: impl FnOnce(&mut SummaryLedger) -> T) -> Option<T> {
    #[cfg(test)]
    if TEST_SUMMARY_LEDGER.with(|l| l.borrow().is_some()) {
        return TEST_SUMMARY_LEDGER.with(|l| Some(f(l.borrow_mut().as_mut().unwrap())));
    }
    let mut ledger = SUMMARY_RECEIPTS.get_or_init(Default::default).lock().ok()?;
    Some(f(&mut ledger))
}

pub(crate) fn message_digest(value: &Value) -> Option<String> {
    // Check structure/size before serialization or cloning.
    fn bounded(v: &Value, depth: usize, fields: &mut usize, bytes: &mut usize) -> bool {
        *fields += 1;
        if depth > MAX_DEPTH || *fields > MAX_LINES {
            return false;
        }
        match v {
            Value::String(s) => *bytes = bytes.saturating_add(s.len()),
            Value::Array(a) => {
                for v in a {
                    if !bounded(v, depth + 1, fields, bytes) {
                        return false;
                    }
                }
            }
            Value::Object(o) => {
                for (k, v) in o {
                    *bytes = bytes.saturating_add(k.len());
                    if !bounded(v, depth + 1, fields, bytes) {
                        return false;
                    }
                }
            }
            _ => *bytes = bytes.saturating_add(8),
        }
        *bytes <= MAX_BYTES
    }
    if !bounded(value, 0, &mut 0, &mut 0) {
        return None;
    }
    Some(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).ok()?)
    ))
}

pub(crate) fn remember(session: &str, slot: &SourceSlot) {
    if session.len() > 256 || slot.entry_id.len() > 256 || slot.observer_id.len() > 256 {
        return;
    }
    with_slot_ledger(|ledger| {
        ledger.retain(|(s, old)| s != session || old.entry_id != slot.entry_id);
        if ledger.len() == MAX_LINES {
            ledger.pop_front();
        }
        ledger.push_back((session.to_owned(), slot.clone()));
    });
}

pub(crate) fn replay_slot(
    session: &str,
    entry_id: &str,
    message: &Value,
    index: usize,
) -> Option<SourceSlot> {
    let digest = message_digest(message)?;
    let mut slot = with_slot_ledger(|ledger| {
        ledger
            .iter()
            .find(|(s, r)| s == session && r.entry_id == entry_id && r.message_sha256 == digest)
            .map(|(_, r)| r.clone())
    })??;
    slot.message_index = index;
    slot.source = "replay_memory".into();
    Some(slot)
}

/// Synthetic-only attestation by the audited producer, not SDK collector activation.
/// Join an independently authenticated Memory output by output_sha256; then join
/// its normalized append to a SourceSlot by observer_id/ordinal/message_sha256.
/// Missing receipts are unavailable. Consumers deduplicate receipt_id and bind
/// session/turn/step before accepting; never infer invocation from registry size.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct MemoryReceipt {
    pub version: u32,
    pub receipt_id: String,
    pub producer_instance: String,
    pub session_id: String,
    pub turn_id: String,
    pub step: u64,
    pub observer_id: String,
    pub output_sha256: String,
    pub appends: Vec<AppendReceipt>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct AppendReceipt {
    pub append_ordinal: usize,
    pub message_sha256: String,
}

pub(crate) const MEMORY_RECEIPT: &str = "harness::on-synthetic-memory-receipt";

/// Only ThroughObservers' trusted in-process path calls this after BOTH owner
/// checks and a successful observer call. No handler accepts caller authority.
pub(crate) async fn emit_memory_receipt(
    iii: &IIIClient,
    input: &Value,
    output: &Value,
    source: &AppendSource,
) {
    if capture_mode() != CaptureMode::SyntheticIsolation || !iii.is_observed(MEMORY_RECEIPT) {
        return;
    }
    let Some(output_sha256) = message_digest(output) else {
        return;
    };
    let (Some(session), Some(turn), Some(step)) = (
        input["session_id"].as_str(),
        input["turn_id"].as_str(),
        input["step"].as_u64(),
    ) else {
        return;
    };
    if session.len() > 256 || turn.len() > 256 || source.observer_id.len() > 256 {
        return;
    }
    let Some(messages) = output
        .pointer("/mutations/append_messages")
        .and_then(Value::as_array)
    else {
        return;
    };
    if messages.len() > MAX_LINES {
        return;
    }
    let appends: Option<Vec<_>> = messages
        .iter()
        .enumerate()
        .map(|(append_ordinal, value)| {
            let message = crate::window::replayed(value)?;
            Some(AppendReceipt {
                append_ordinal,
                message_sha256: message_digest(&message)?,
            })
        })
        .collect();
    let Some(appends) = appends else { return };
    let receipt = MemoryReceipt {
        version: 1,
        receipt_id: uuid::Uuid::new_v4().to_string(),
        producer_instance: INSTANCE
            .get_or_init(|| uuid::Uuid::new_v4().to_string())
            .clone(),
        session_id: session.into(),
        turn_id: turn.into(),
        step,
        observer_id: source.observer_id.clone(),
        output_sha256,
        appends,
    };
    bounded_telemetry(iii.call(CallRequest {
        function_id: MEMORY_RECEIPT.into(),
        payload: serde_json::to_value(receipt).unwrap_or(Value::Null),
        action: Some(CallAction::Void),
        timeout_ms: Some(50),
    }))
    .await;
}

pub(crate) const REQUEST: &str = "harness::on-generation-request";
pub(crate) const RESPONSE: &str = "harness::on-generation-response";
const MAX_LINES: usize = 2048;
const MAX_BYTES: usize = 1024 * 1024;
const MAX_DEPTH: usize = 16;
static NEXT: AtomicU64 = AtomicU64::new(0);
static SEQUENCE: AtomicU64 = AtomicU64::new(1);
static INSTANCE: OnceLock<String> = OnceLock::new();
static CAPTURE_MODE: OnceLock<CaptureMode> = OnceLock::new();

/// Internal immutable capability, never deserialized from caller JSON.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CaptureMode {
    #[default]
    Disabled,
    SyntheticIsolation,
}

impl CaptureMode {
    pub(crate) fn parse(value: Option<&std::ffi::OsStr>) -> Self {
        if value == Some(std::ffi::OsStr::new("1")) {
            Self::SyntheticIsolation
        } else {
            Self::Disabled
        }
    }
}

pub(crate) fn capture_mode_at_boot() -> CaptureMode {
    *CAPTURE_MODE.get_or_init(|| {
        CaptureMode::parse(std::env::var_os("III_HARNESS_SYNTHETIC_GENERATION_CAPTURE").as_deref())
    })
}

#[cfg(test)]
tokio::task_local! {
    // Explicit per-task injection, never an automatic test-build opt-in.
    static TEST_CAPTURE_MODE: CaptureMode;
}

/// Before registration, capture is denied without reading the environment.
pub(crate) fn capture_mode() -> CaptureMode {
    #[cfg(test)]
    if let Ok(mode) = TEST_CAPTURE_MODE.try_with(|mode| *mode) {
        return mode;
    }
    CAPTURE_MODE.get().copied().unwrap_or_default()
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct LineWitness {
    pub role: String,
    /// Structured numeric path: message index, then content block indexes.
    /// Empty for the flattened system prompt.
    pub location: Vec<usize>,
    pub line: usize,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct GenerationEvent {
    pub version: u32,
    pub producer_instance: String,
    pub generation_id: String,
    /// Internal deterministic output target; not supplied by caller metadata.
    pub assistant_entry_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub step: u64,
    /// Process-wide dispatch ordinal, NOT a retry count parsed from request_id.
    /// Distinguishes every retry, including redelivery of the same step.
    pub dispatch_index: u64,
    pub sequence: u64,
    /// request, success, error, cancelled. No raw failure text.
    pub phase: String,
    pub complete: bool,
    pub message_count: usize,
    pub text_bytes: usize,
    pub lines: Vec<LineWitness>,
    /// v2: exact final serialized message slots authenticated by this producer.
    pub sources: Vec<SourceSlot>,
    #[serde(deserialize_with = "required_summary")]
    #[schemars(required, schema_with = "Option::<SummaryWitness>::json_schema")]
    pub summary: Option<SummaryWitness>,
    /// Separate from byte completeness: unsupported origins fail closed.
    pub origins_complete: bool,
}

/// Exact algorithm shared with consumers: split with Rust str::lines (LF,
/// CRLF); trim Unicode whitespace, remove ONE leading '- ', '* ', or '+ '
/// bullet, trim again, preserve case/spelling and hash the WHOLE nonempty line
/// as UTF-8 SHA-256 lowercase hex. No substring matching or Unicode folding.
pub(crate) fn line_digest(line: &str) -> Option<String> {
    let line = line.trim();
    let line = ["- ", "* ", "+ "]
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix))
        .unwrap_or(line)
        .trim();
    (!line.is_empty()).then(|| format!("{:x}", Sha256::digest(line.as_bytes())))
}

fn required_summary<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<SummaryWitness>, D::Error> {
    Option::deserialize(deserializer)
}

impl GenerationEvent {
    pub(crate) fn new(session_id: &str, turn_id: &str, step: u64) -> Self {
        let dispatch_index = NEXT.fetch_add(1, Ordering::Relaxed);
        Self {
            version: 2,
            producer_instance: INSTANCE
                .get_or_init(|| uuid::Uuid::new_v4().to_string())
                .clone(),
            generation_id: uuid::Uuid::new_v4().to_string(),
            assistant_entry_id: crate::ids::assistant_entry_id(turn_id, step),
            session_id: session_id.to_owned(),
            turn_id: turn_id.to_owned(),
            step,
            dispatch_index,
            sequence: 0,
            phase: "not_dispatched".into(),
            complete: true,
            message_count: 0,
            text_bytes: 0,
            lines: Vec::new(),
            sources: Vec::new(),
            summary: None,
            origins_complete: false,
        }
    }

    fn text(&mut self, text: &str, role: &str, location: &[usize]) {
        if location.len() > MAX_DEPTH || self.text_bytes.saturating_add(text.len()) > MAX_BYTES {
            self.complete = false;
            return;
        }
        self.text_bytes += text.len();
        for (line, text) in text.lines().enumerate() {
            if self.lines.len() == MAX_LINES {
                self.complete = false;
                return;
            }
            if let Some(sha256) = line_digest(text) {
                self.lines.push(LineWitness {
                    role: role.into(),
                    location: location.to_vec(),
                    line,
                    sha256,
                });
            }
        }
    }

    fn content(&mut self, content: &Value, role: &str, path: &mut Vec<usize>) {
        if path.len() > MAX_DEPTH {
            self.complete = false;
            return;
        }
        if let Some(text) = content.as_str() {
            self.text(text, role, path);
            return;
        }
        let Some(blocks) = content.as_array() else {
            self.complete = false;
            return;
        };
        for (i, block) in blocks.iter().enumerate() {
            path.push(i);
            match block["type"].as_str() {
                Some("text") => match block["text"].as_str() {
                    Some(text) => self.text(text, role, path),
                    None => self.complete = false,
                },
                Some("function_result") => self.content(&block["content"], role, path),
                // Known non-text content is deliberately outside this witness.
                Some("image" | "thinking" | "function_call") => {}
                _ => self.complete = false,
            }
            path.pop();
        }
    }

    pub(crate) fn capture(&mut self, payload: &Value) {
        if let Some(system) = payload["system_prompt"].as_str() {
            self.text(system, "system", &[]);
        }
        // Sections are a cache representation of the flat system prompt. If
        // they differ, do not certify any effective system interpretation.
        if let Some(sections) = payload["system_sections"].as_array() {
            if sections.len() > MAX_LINES
                || sections
                    .iter()
                    .try_fold(0usize, |sum, section| {
                        sum.checked_add(section["text"].as_str()?.len())?
                            .checked_add(2)
                    })
                    .is_none_or(|bytes| bytes > MAX_BYTES)
            {
                self.complete = false;
                return;
            }
            let flat: Option<Vec<&str>> = sections.iter().map(|s| s["text"].as_str()).collect();
            if flat.map(|s| s.join("\n\n")).as_deref() != payload["system_prompt"].as_str() {
                self.complete = false;
            }
        }
        let Some(messages) = payload["messages"].as_array() else {
            self.complete = false;
            return;
        };
        if messages.len() > MAX_LINES {
            self.complete = false;
            return;
        }
        self.message_count = messages.len();
        for (i, message) in messages.iter().enumerate() {
            let role = match message["role"].as_str() {
                Some(role @ ("user" | "assistant" | "function_result" | "custom" | "system")) => {
                    role
                }
                _ => {
                    self.complete = false;
                    "unknown"
                }
            };
            self.content(&message["content"], role, &mut vec![i]);
        }
        if self.lines.is_empty() {
            self.complete = false;
        }
    }

    pub(crate) fn bind_sources(&mut self, payload: &Value, hints: &SourceHints) {
        self.origins_complete = hints.complete && !hints.invalid;
        if hints.invalid {
            self.complete = false;
            return;
        }
        if hints.slots.len() > MAX_LINES {
            self.complete = false;
            self.origins_complete = false;
            return;
        }
        let Some(messages) = payload["messages"].as_array() else {
            self.complete = false;
            return;
        };
        if let Some(summary) = &hints.summary {
            if summary.inputs.len() > MAX_LINES
                || summary.lines.len() > MAX_LINES
                || payload["system_prompt"]
                    .as_str()
                    .and_then(|system| valid_span(system, &summary.span))
                    .is_none()
            {
                self.complete = false;
                self.origins_complete = false;
                return;
            }
            self.summary = Some(summary.clone());
        }
        let mut seen = std::collections::HashSet::new();
        for slot in &hints.slots {
            if slot.entry_id.len() > 256
                || slot.observer_id.len() > 256
                || !matches!(slot.source.as_str(), "fresh_memory" | "replay_memory")
                || slot.message_sha256.len() != 64
                || !seen.insert(slot.message_index)
                || messages
                    .get(slot.message_index)
                    .and_then(message_digest)
                    .as_deref()
                    != Some(slot.message_sha256.as_str())
            {
                self.complete = false;
                self.origins_complete = false;
                self.sources.clear();
                self.summary = None;
                return;
            }
            self.sources.push(slot.clone());
        }
    }

    pub(crate) async fn emit(&mut self, iii: &IIIClient, id: &str) {
        self.emit_with(iii.is_observed(id), id, |request| iii.call(request))
            .await;
    }

    /// Injectable transport, not a caller capability. Tests exercise this
    /// same serialization/sequence/deadline path using a bounded channel.
    async fn emit_with<F, Fut, T, E>(&mut self, observed: bool, id: &str, send: F)
    where
        F: FnOnce(CallRequest) -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
    {
        if !observed {
            return;
        }
        self.sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        // Bounded wait and no raw errors in telemetry. Failure is unavailable,
        // never a generation failure; a consumer must see both events.
        bounded_telemetry(send(CallRequest {
            function_id: id.into(),
            payload: serde_json::to_value(self).unwrap_or(Value::Null),
            action: Some(CallAction::Void),
            timeout_ms: Some(50),
        }))
        .await;
    }

    pub(crate) async fn finish(&mut self, iii: &IIIClient, phase: &str) {
        self.phase = phase.into();
        self.lines.clear();
        self.sources.clear();
        self.summary = None;
        self.emit(iii, RESPONSE).await;
    }
}

/// The SDK currently enqueues Void on an unbounded queue: 50 ms bounds the
/// awaited operation, not queue delivery, backlog size or provider receipt.
/// Kept separate so a saturated/failing transport future is exercised without
/// mutating SDK internals or requiring a live engine.
async fn bounded_telemetry<F, T, E>(send: F)
where
    F: std::future::Future<Output = Result<T, E>>,
{
    let _ = tokio::time::timeout(std::time::Duration::from_millis(50), send).await;
}

#[cfg(test)]
#[path = "generation_capture_tests.rs"]
mod tests;
