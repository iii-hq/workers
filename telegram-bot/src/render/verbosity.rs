use crate::config::Verbosity;
use crate::types::{AgentMessage, ContentBlock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessagePhase {
    Empty,
    ThinkingOnly,
    Answering,
}

pub fn message_phase(content: &[ContentBlock]) -> MessagePhase {
    let has_text = content
        .iter()
        .any(|b| matches!(b, ContentBlock::Text { text } if !text.is_empty()));
    let has_thinking = content
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { text } if !text.is_empty()));
    match (has_text, has_thinking) {
        (false, true) => MessagePhase::ThinkingOnly,
        (true, _) => MessagePhase::Answering,
        _ => MessagePhase::Empty,
    }
}

impl PartialOrd for Verbosity {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Verbosity {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let rank = |v: Verbosity| match v {
            Verbosity::None => 0,
            Verbosity::Minimal => 1,
            Verbosity::High => 2,
            Verbosity::Debug => 3,
        };
        rank(*self).cmp(&rank(*other))
    }
}

pub fn render_assistant_message(message: &AgentMessage, verbosity: Verbosity) -> Option<String> {
    if message.role != "assistant" {
        return None;
    }
    let parts = render_content_blocks(&message.content, verbosity, false);
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("\n\n"))
}

pub fn render_answer_text(message: &AgentMessage, verbosity: Verbosity) -> Option<String> {
    if message.role != "assistant" {
        return None;
    }
    let parts = render_content_blocks(&message.content, verbosity, false);
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("\n\n"))
}

/// Raw thinking block text for RichBlockThinking drafts (not gated by verbosity).
pub fn render_thinking_text(message: &AgentMessage) -> Option<String> {
    if message.role != "assistant" {
        return None;
    }
    let mut out = Vec::new();
    for block in &message.content {
        if let ContentBlock::Thinking { text } = block {
            if !text.is_empty() {
                out.push(text.clone());
            }
        }
    }
    if out.is_empty() {
        return None;
    }
    Some(out.join("\n\n"))
}

pub fn render_function_result_message(
    message: &AgentMessage,
    verbosity: Verbosity,
) -> Option<String> {
    if message.role != "function_result" {
        return None;
    }
    if verbosity != Verbosity::Debug {
        return None;
    }
    let parts = render_content_blocks(&message.content, verbosity, true);
    if parts.is_empty() {
        return None;
    }
    Some(format!("function result:\n{}", parts.join("\n")))
}

fn verbosity_at_least(have: Verbosity, need: Verbosity) -> bool {
    have >= need
}

fn render_content_blocks(
    blocks: &[ContentBlock],
    verbosity: Verbosity,
    force_all_text: bool,
) -> Vec<String> {
    let mut out = Vec::new();

    for block in blocks {
        match block {
            ContentBlock::Text { text } if !text.is_empty() => {
                out.push(text.clone());
            }
            ContentBlock::FunctionCall {
                function_id,
                arguments,
                ..
            } if verbosity_at_least(verbosity, Verbosity::High) => {
                let args = truncate_json(arguments, 400);
                out.push(format!("⚙️ `{function_id}`\n{args}"));
            }
            ContentBlock::Other if force_all_text => {}
            _ => {}
        }
    }
    out
}

fn truncate_json(value: &serde_json::Value, max: usize) -> String {
    crate::text::truncate_ellipsis(&value.to_string(), max)
}

pub fn turn_status_message(
    status: &str,
    reason: Option<&str>,
    result_error: Option<&str>,
) -> String {
    match status {
        "failed" => {
            let msg = result_error.or(reason).unwrap_or("turn failed");
            format!("❌ {msg}")
        }
        "cancelled" => {
            let msg = reason.unwrap_or("turn cancelled");
            format!("⏹ {msg}")
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ContentBlock;
    use serde_json::json;

    fn assistant(content: Vec<ContentBlock>) -> AgentMessage {
        AgentMessage {
            role: "assistant".into(),
            content,
        }
    }

    #[test]
    fn none_verbosity_text_only() {
        let msg = assistant(vec![
            ContentBlock::Text {
                text: "hello".into(),
            },
            ContentBlock::Thinking { text: "hmm".into() },
        ]);
        let out = render_assistant_message(&msg, Verbosity::None).unwrap();
        assert_eq!(out, "hello");
    }

    #[test]
    fn thinking_extracted_regardless_of_verbosity() {
        let msg = assistant(vec![ContentBlock::Thinking {
            text: "plan".into(),
        }]);
        let thinking = render_thinking_text(&msg).unwrap();
        assert_eq!(thinking, "plan");
    }

    #[test]
    fn answer_excludes_thinking_blocks() {
        let msg = assistant(vec![
            ContentBlock::Thinking {
                text: "plan".into(),
            },
            ContentBlock::Text {
                text: "answer".into(),
            },
        ]);
        let thinking = render_thinking_text(&msg).unwrap();
        let answer = render_answer_text(&msg, Verbosity::None).unwrap();
        assert_eq!(thinking, "plan");
        assert_eq!(answer, "answer");
    }

    #[test]
    fn message_phase_detection() {
        assert_eq!(
            message_phase(&[ContentBlock::Thinking { text: "x".into() }]),
            MessagePhase::ThinkingOnly
        );
        assert_eq!(
            message_phase(&[
                ContentBlock::Thinking { text: "x".into() },
                ContentBlock::Text { text: "y".into() }
            ]),
            MessagePhase::Answering
        );
    }

    #[test]
    fn high_includes_function_call() {
        let msg = assistant(vec![ContentBlock::FunctionCall {
            id: "c1".into(),
            function_id: "shell::exec".into(),
            arguments: json!({ "cmd": "ls" }),
        }]);
        let out = render_assistant_message(&msg, Verbosity::High).unwrap();
        assert!(out.contains("shell::exec"));
    }

    #[test]
    fn debug_renders_function_result() {
        let msg = AgentMessage {
            role: "function_result".into(),
            content: vec![ContentBlock::Text { text: "ok".into() }],
        };
        let out = render_function_result_message(&msg, Verbosity::Debug).unwrap();
        assert!(out.contains("ok"));
    }
}
