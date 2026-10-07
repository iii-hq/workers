//! The text the agent reads: the `<mentions>` block that resolves the
//! mentions in the user's messages and the `<mention_providers>` index, plus
//! reading them back out of a transcript (so the hook never says the same
//! thing twice). Pure — no engine.

use std::collections::BTreeSet;

use mention_contract::{format_mention, parse_mentions, parse_prose_mentions, MentionRef};
use serde::Serialize;
use serde_json::Value;

use super::registry::Provider;

pub const MENTIONS_TAG: &str = "<mentions>";
pub const PROVIDERS_TAG: &str = "<mention_providers>";
/// A summary longer than this is cut (a provider's summary is one line).
const MAX_SUMMARY_CHARS: usize = 400;

/// What became of one mention.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MentionStatus {
    Resolved,
    NotFound,
    UnknownProvider,
    Error,
}

/// The pre-verified call that returns a mentioned item's full details.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct DetailsCall {
    pub function_id: String,
    pub payload: Value,
}

/// One mention, resolved (or not).
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct ResolvedMention {
    /// The canonical token (`@<name>(id="<id>")`).
    pub token: String,
    pub name: String,
    pub id: String,
    pub status: MentionStatus,
    /// What the provider calls this kind of item ("Tickets").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The agent-facing line, when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<DetailsCall>,
    /// Why it did not resolve, when it errored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn text_blocks(message: &Value) -> Vec<&str> {
    match message.get("content") {
        Some(Value::String(text)) => vec![text.as_str()],
        Some(Value::Array(parts)) => parts
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect(),
        _ => Vec::new(),
    }
}

fn is_user(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("user")
}

/// Text the console or a hook attached to a user message rather than what
/// the user wrote: file windows, skill bodies, other hooks' blocks. Tokens
/// inside them are not the user's mentions.
fn is_attached_block(text: &str) -> bool {
    let head = text.trim_start();
    head.starts_with('<')
        && [
            "<attached-file",
            "<skill",
            "<memory",
            "<discovery_assist",
            "<preloaded_functions",
            "<available_skills",
            MENTIONS_TAG,
            PROVIDERS_TAG,
        ]
        .iter()
        .any(|tag| head.starts_with(tag))
}

/// Keys (`name`, `id`) every earlier `<mentions>` block already resolved.
pub fn already_resolved(messages: &[Value]) -> BTreeSet<(String, String)> {
    let mut keys = BTreeSet::new();
    for message in messages.iter().filter(|m| is_user(m)) {
        for text in text_blocks(message) {
            if text.trim_start().starts_with(MENTIONS_TAG) {
                keys.extend(parse_mentions(text).iter().map(MentionRef::key));
            }
        }
    }
    keys
}

/// The mentions users wrote, oldest first, each item once, minus `skip`.
/// At most `limit`, keeping the newest when there are more.
pub fn pending_mentions(
    messages: &[Value],
    skip: &BTreeSet<(String, String)>,
    limit: usize,
) -> Vec<MentionRef> {
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    for message in messages.iter().filter(|m| is_user(m)) {
        for text in text_blocks(message) {
            if is_attached_block(text) {
                continue;
            }
            for mention in parse_prose_mentions(text) {
                let key = mention.key();
                if skip.contains(&key) || !seen.insert(key) {
                    continue;
                }
                found.push(mention);
            }
        }
    }
    let excess = found.len().saturating_sub(limit);
    found.split_off(excess)
}

/// Provider names in the newest `<mention_providers>` block, if any.
pub fn advertised_providers(messages: &[Value]) -> Option<BTreeSet<String>> {
    messages
        .iter()
        .rev()
        .filter(|m| is_user(m))
        .flat_map(text_blocks)
        .find(|text| text.trim_start().starts_with(PROVIDERS_TAG))
        .map(|text| {
            text.lines()
                .filter_map(|line| line.trim().strip_prefix("- @"))
                .filter_map(|rest| rest.split_whitespace().next())
                .map(str::to_string)
                .collect()
        })
}

fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

/// The `<mention_providers>` index: which names the agent may write.
pub fn providers_block(providers: &[Provider]) -> String {
    let mut block = String::from(PROVIDERS_TAG);
    let example = providers
        .first()
        .map(|provider| format!(", e.g. @{}(id=\"…\")", provider.descriptor.name))
        .unwrap_or_default();
    block.push_str(&format!(
        "\nItems you can reference in a reply as @<name>(id=\"<id>\") — both quotes included{example} — with an id a function returned (never a guessed one); the console shows each as a rich link.\n"
    ));
    for provider in providers {
        let descriptor = &provider.descriptor;
        block.push_str(&format!("- @{} — {}", descriptor.name, descriptor.label));
        if let Some(description) = descriptor.description.as_deref().filter(|d| !d.is_empty()) {
            block.push_str(": ");
            block.push_str(&clip(description, 160));
        }
        block.push('\n');
    }
    block.push_str("</mention_providers>");
    block
}

/// The `<mentions>` block for newly seen mentions, cut to `budget_chars`
/// (whole entries only; the rest are counted, not dropped silently).
pub fn mentions_block(mentions: &[ResolvedMention], budget_chars: usize) -> String {
    let mut block = String::from(MENTIONS_TAG);
    block.push_str(
        "\nItems the user's message references, resolved by the workers that own them. A `details` call is pre-verified: call it directly (no discovery, no contract lookup), and only when the summary is not enough. To refer to one of these items in your reply, copy its token exactly as written here.\n",
    );
    let mut omitted = 0usize;
    for mention in mentions {
        let entry = mention_entry(mention);
        if block.len() + entry.len() > budget_chars {
            omitted += 1;
            continue;
        }
        block.push_str(&entry);
    }
    if omitted > 0 {
        block.push_str(&format!(
            "- {omitted} more mention(s) not shown; resolve them through their workers' get-by-id functions.\n"
        ));
    }
    block.push_str("</mentions>");
    block
}

fn mention_entry(mention: &ResolvedMention) -> String {
    let token = format_mention(&mention.name, &mention.id);
    match mention.status {
        MentionStatus::Resolved => {
            let summary = clip(mention.summary.as_deref().unwrap_or(""), MAX_SUMMARY_CHARS);
            let mut entry = format!("- {token} — {summary}\n");
            if let Some(details) = &mention.details {
                entry.push_str(&format!(
                    "  details: {} {}\n",
                    details.function_id, details.payload
                ));
            }
            entry
        }
        MentionStatus::NotFound => format!(
            "- {token} — not found: the {} worker knows no such id\n",
            mention.name
        ),
        MentionStatus::UnknownProvider => format!(
            "- {token} — no installed worker provides @{} mentions\n",
            mention.name
        ),
        MentionStatus::Error => format!(
            "- {token} — could not be resolved right now ({})\n",
            clip(mention.error.as_deref().unwrap_or("unknown error"), 160)
        ),
    }
}
