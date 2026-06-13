//! Summariser prompt construction (context-manager.md §
//! context::compact): the fixed Markdown template, previous-summary
//! anchoring (update-in-place so summaries converge instead of
//! growing), media stripping, and the `# Conversation summary` system
//! prompt section `assemble` renders.

use crate::types::{AgentMessage, ContentBlock};

/// The fixed summary structure (spec: Goal / Constraints / Progress /
/// Key Decisions / Actions Taken / Next Steps / Critical Context /
/// Relevant Files).
pub const SUMMARY_TEMPLATE: &str = r#"Output exactly the Markdown structure shown inside <template> and keep the section order unchanged. Do not include the <template> tags in your response.
<template>
## Goal
- [single-sentence task summary]

## Constraints & Preferences
- [user constraints, preferences, specs, or "(none)"]

## Progress
### Done
- [completed work or "(none)"]

### In Progress
- [current work or "(none)"]

### Blocked
- [blockers or "(none)"]

## Key Decisions
- [decision and why, or "(none)"]

## Actions Taken
- [notable function calls with their arguments (terse), or "(none)"]

## Next Steps
- [ordered next actions or "(none)"]

## Critical Context
- [important technical facts, errors, open questions, or "(none)"]

## Relevant Files
- [file or directory path: why it matters, or "(none)"]
</template>

Rules:
- Keep every section, even when empty.
- Use terse bullets, not prose paragraphs.
- Preserve exact file paths, commands, error strings, and identifiers when known.
- Do not mention the summary process or that context was compacted."#;

/// System prompt for the summariser turn. With a `previous_summary`
/// the anchor instructs an update-in-place merge instead of a restart.
pub fn build_system_prompt(previous_summary: Option<&str>) -> String {
    let anchor = match previous_summary {
        Some(prior) => format!(
            "Update the anchored summary below using the conversation history above.\n\
             Preserve still-true details, remove stale details, and merge in the new facts.\n\
             <previous-summary>\n{prior}\n</previous-summary>"
        ),
        None => "Create a new anchored summary from the conversation history above.".to_string(),
    };
    format!("{anchor}\n\n{SUMMARY_TEMPLATE}")
}

/// User prompt: the head messages rendered as a `<conversation>` block
/// (text blocks verbatim, function calls as terse one-liners).
pub fn render_user_prompt(older: &[AgentMessage]) -> String {
    let mut out = String::from(
        "Summarise the conversation below following the system-prompt structure exactly. \
         Keep identifiers verbatim.\n\n<conversation>\n",
    );
    for message in older {
        out.push_str(&format!("\n[{}]\n", role_label(message)));
        for block in message.content() {
            match block {
                ContentBlock::Text { text } => {
                    out.push_str(text);
                    out.push('\n');
                }
                ContentBlock::FunctionCall {
                    function_id,
                    arguments,
                    ..
                } => {
                    out.push_str(&format!(
                        "[tool_call] {function_id} {}\n",
                        serde_json::to_string(arguments).unwrap_or_default()
                    ));
                }
                _ => {}
            }
        }
    }
    out.push_str("</conversation>\n");
    out
}

fn role_label(message: &AgentMessage) -> &'static str {
    match message {
        AgentMessage::User { .. } => "user",
        AgentMessage::Assistant { .. } => "assistant",
        AgentMessage::FunctionResult { .. } => "function_result",
        AgentMessage::Custom { .. } => "custom",
    }
}

/// Copy of `messages` fit for the summariser: images become
/// `[image stripped]` placeholders everywhere; `function_result` text
/// blocks are truncated to `max_output_chars`.
pub fn strip_media(messages: &[AgentMessage], max_output_chars: usize) -> Vec<AgentMessage> {
    messages
        .iter()
        .map(|message| {
            let truncate = matches!(message, AgentMessage::FunctionResult { .. });
            let mut stripped = message.clone();
            let content = stripped
                .content()
                .iter()
                .map(|block| {
                    let block = strip_image(block);
                    if truncate {
                        truncate_text(block, max_output_chars)
                    } else {
                        block
                    }
                })
                .collect();
            stripped.set_content(content);
            stripped
        })
        .collect()
}

fn strip_image(block: &ContentBlock) -> ContentBlock {
    match block {
        ContentBlock::Image { .. } => ContentBlock::Text {
            text: "[image stripped]".to_string(),
        },
        other => other.clone(),
    }
}

fn truncate_text(block: ContentBlock, max: usize) -> ContentBlock {
    match block {
        ContentBlock::Text { text } if text.len() > max => {
            let mut cut = max;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            ContentBlock::Text {
                text: format!("{}... [truncated]", &text[..cut]),
            }
        }
        other => other,
    }
}

/// Heading under which `assemble` renders the conversation summary into
/// the returned system prompt (context-manager.md § The compaction
/// round trip).
pub const SUMMARY_HEADING: &str = "# Conversation summary";

/// Base system prompt + the summary section. An empty base yields just
/// the section; no summary yields just the base.
pub fn render_system_prompt(base: Option<&str>, summary: Option<&str>) -> String {
    let base = base.unwrap_or("").trim_end();
    match summary {
        None => base.to_string(),
        Some(summary) => {
            if base.is_empty() {
                format!("{SUMMARY_HEADING}\n\n{summary}")
            } else {
                format!("{base}\n\n{SUMMARY_HEADING}\n\n{summary}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(v: serde_json::Value) -> AgentMessage {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn fresh_prompt_instructs_creation_and_carries_every_section() {
        let prompt = build_system_prompt(None);
        assert!(prompt.starts_with("Create a new anchored summary"));
        for section in [
            "## Goal",
            "## Constraints & Preferences",
            "## Progress",
            "## Key Decisions",
            "## Actions Taken",
            "## Next Steps",
            "## Critical Context",
            "## Relevant Files",
        ] {
            assert!(prompt.contains(section), "missing {section}");
        }
    }

    #[test]
    fn previous_summary_switches_to_update_mode() {
        let prompt = build_system_prompt(Some("## Goal\n- ship it"));
        assert!(prompt.starts_with("Update the anchored summary"));
        assert!(prompt.contains("<previous-summary>\n## Goal\n- ship it\n</previous-summary>"));
    }

    #[test]
    fn user_prompt_renders_text_and_tool_calls() {
        let messages = vec![
            message(json!({
                "role": "user", "content": [{ "type": "text", "text": "find the bug" }],
                "timestamp": 1
            })),
            message(json!({
                "role": "assistant",
                "content": [{ "type": "function_call", "id": "c1", "function_id": "coder::search",
                              "arguments": { "q": "panic" } }],
                "stop_reason": "function_call", "model": "m", "provider": "p", "timestamp": 2
            })),
        ];
        let prompt = render_user_prompt(&messages);
        assert!(prompt.contains("[user]\nfind the bug"));
        assert!(prompt.contains("[tool_call] coder::search {\"q\":\"panic\"}"));
        assert!(prompt.contains("<conversation>") && prompt.contains("</conversation>"));
    }

    #[test]
    fn strip_media_replaces_images_and_truncates_outputs() {
        let messages = vec![
            message(json!({
                "role": "user",
                "content": [{ "type": "image", "mime": "image/png", "data": "AAAA" }],
                "timestamp": 1
            })),
            message(json!({
                "role": "function_result", "function_call_id": "c1", "function_id": "f",
                "content": [{ "type": "text", "text": "x".repeat(50) }], "timestamp": 2
            })),
        ];
        let stripped = strip_media(&messages, 10);
        assert_eq!(
            stripped[0].content(),
            &[ContentBlock::Text {
                text: "[image stripped]".into()
            }]
        );
        let ContentBlock::Text { text } = &stripped[1].content()[0] else {
            panic!("expected text");
        };
        assert_eq!(text, &format!("{}... [truncated]", "x".repeat(10)));
        // Originals untouched (the head is summarised; the transcript
        // the caller holds must never be mutated).
        assert!(matches!(
            messages[0].content()[0],
            ContentBlock::Image { .. }
        ));
    }

    #[test]
    fn user_text_is_never_truncated_by_strip_media() {
        let messages = vec![message(json!({
            "role": "user", "content": [{ "type": "text", "text": "y".repeat(100) }],
            "timestamp": 1
        }))];
        let stripped = strip_media(&messages, 10);
        let ContentBlock::Text { text } = &stripped[0].content()[0] else {
            panic!("expected text");
        };
        assert_eq!(text.len(), 100);
    }

    #[test]
    fn render_system_prompt_combinations() {
        assert_eq!(render_system_prompt(Some("Base."), None), "Base.");
        assert_eq!(render_system_prompt(None, None), "");
        assert_eq!(
            render_system_prompt(Some("Base."), Some("S")),
            "Base.\n\n# Conversation summary\n\nS"
        );
        assert_eq!(
            render_system_prompt(None, Some("S")),
            "# Conversation summary\n\nS"
        );
    }
}
