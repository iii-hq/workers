//! Terminal-frame synthesis (spec § close-without-terminal, § router::abort).
use crate::types::events::{AssistantMessageEvent, ErrorKind, StopReason, Usage};
use crate::types::messages::{AssistantMessage, AssistantRoleTag};

pub fn empty_partial(model: &str, provider: &str, now_ms: i64) -> AssistantMessage {
    AssistantMessage {
        role: AssistantRoleTag::Assistant,
        content: vec![],
        stop_reason: StopReason::End,
        native_stop_reason: None,
        error_message: None,
        error_kind: None,
        warnings: None,
        usage: None,
        model: model.to_string(),
        provider: provider.to_string(),
        timestamp: now_ms,
    }
}

pub fn synthesize_error(
    partial: Option<&AssistantMessage>,
    model: &str,
    provider: &str,
    message: &str,
    error_kind: ErrorKind,
    usage: Option<Usage>,
    now_ms: i64,
) -> AssistantMessageEvent {
    let mut base = partial
        .cloned()
        .unwrap_or_else(|| empty_partial(model, provider, now_ms));
    base.stop_reason = StopReason::Error;
    base.error_kind = Some(error_kind);
    base.error_message = Some(message.to_string());
    base.usage = usage.or(base.usage); // partial usage survives a synthesized terminal
    base.timestamp = now_ms;
    AssistantMessageEvent::Error { error: base }
}

pub fn synthesize_aborted(
    partial: Option<&AssistantMessage>,
    model: &str,
    provider: &str,
    usage: Option<Usage>,
    now_ms: i64,
) -> AssistantMessageEvent {
    let mut base = partial
        .cloned()
        .unwrap_or_else(|| empty_partial(model, provider, now_ms));
    base.stop_reason = StopReason::Aborted;
    base.usage = usage.or(base.usage);
    base.timestamp = now_ms;
    AssistantMessageEvent::Done { message: base }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::events::{ErrorKind, StopReason, Usage};

    #[test]
    fn error_terminal_keeps_partial_content_transient_kind_and_partial_usage() {
        let partial = empty_partial("m", "p", 1);
        let ev = synthesize_error(
            Some(&partial),
            "m",
            "p",
            "provider p died",
            ErrorKind::Transient,
            Some(Usage {
                input: Some(5),
                ..Default::default()
            }),
            99,
        );
        let crate::types::events::AssistantMessageEvent::Error { error } = ev else {
            panic!("not error")
        };
        assert_eq!(error.stop_reason, StopReason::Error);
        assert_eq!(error.error_kind, Some(ErrorKind::Transient));
        assert_eq!(error.usage.unwrap().input, Some(5));
        assert_eq!(error.timestamp, 99);
    }

    #[test]
    fn aborted_terminal_is_a_done_frame_with_stop_reason_aborted() {
        let ev = synthesize_aborted(None, "m", "p", None, 5);
        let crate::types::events::AssistantMessageEvent::Done { message } = ev else {
            panic!("not done")
        };
        assert_eq!(message.stop_reason, StopReason::Aborted);
        assert!(message.content.is_empty());
    }
}
