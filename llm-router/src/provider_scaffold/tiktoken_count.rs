//! Local prompt token counting for the OpenAI families
//! (`provider::openai::count_tokens`, `provider::openai-codex::count_tokens`)
//! with the tiktoken tokenizers, whose vocabularies are embedded in the
//! binary. These providers expose no metering endpoint, so the count is
//! computed locally; it never runs the model, costs nothing, and needs no
//! network.
//!
//! Only the encoder lives here. Which text a request contributes and what the
//! chat framing costs are in
//! [`crate::provider_scaffold::chat_framing`], shared with every other local
//! counter so the two cannot drift apart.

use tiktoken_rs::{cl100k_base_singleton, o200k_base_singleton, CoreBPE};

use crate::provider_scaffold::chat_framing::count_framed_chat;
use crate::types::messages::AgentMessage;
use crate::types::model::AgentFunction;

// The framing rules and the message walk are shared with every other local
// counter; only the encoder below is tiktoken's own. Re-exported so existing
// callers keep importing them from here.
pub use crate::provider_scaffold::chat_framing::{TOKENS_PER_MESSAGE, TOKENS_REPLY_PRIMING};

/// The count is a local tokenizer estimate, not a provider-metered value.
pub const ESTIMATOR_TIKTOKEN: &str = "tiktoken";

/// Tokenizer for a model id. Namespaced ids (`ns/model`) select on the bare
/// model id. cl100k_base covers the gpt-3.5 and non-o gpt-4 generations
/// (`gpt-4`, `gpt-4-turbo`, …); everything else — gpt-4o, gpt-4.1, gpt-5,
/// the o-series, and unknown-modern ids — is o200k_base.
pub fn encoder_for(model: &str) -> &'static CoreBPE {
    let bare = model.rsplit('/').next().unwrap_or(model);
    if wants_cl100k(bare) {
        cl100k_base_singleton()
    } else {
        o200k_base_singleton()
    }
}

fn wants_cl100k(bare: &str) -> bool {
    if bare.starts_with("gpt-3.5") {
        return true;
    }
    match bare.strip_prefix("gpt-4") {
        Some(rest) => rest.is_empty() || rest.starts_with('-'),
        None => false,
    }
}

fn count(bpe: &CoreBPE, text: &str) -> u64 {
    bpe.encode_ordinary(text).len() as u64
}

/// Estimate prompt tokens for an assembled chat request: reply priming, plus
/// one framed row per system prompt and message, plus each tool's serialized
/// schema. `model` may carry a provider namespace (`codex/gpt-5.2`) — encoder
/// selection strips it internally; the caller decides what to do with the
/// namespace for the rest of its response.
pub fn count_chat_tokens(
    model: &str,
    system_prompt: Option<&str>,
    tools: &[AgentFunction],
    messages: &[AgentMessage],
) -> u64 {
    let bpe = encoder_for(model);
    count_framed_chat(system_prompt, tools, messages, |text| count(bpe, text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_scaffold::chat_framing::message_text;
    use crate::types::content::ContentBlock;
    use crate::types::events::StopReason;
    use crate::types::messages::{AssistantMessage, AssistantRoleTag, UserMessage, UserRoleTag};
    use serde_json::json;

    fn user(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            timestamp: 1,
        })
    }

    #[test]
    fn encoder_selection_by_model_family() {
        assert!(wants_cl100k("gpt-3.5-turbo"));
        assert!(wants_cl100k("gpt-4"));
        assert!(wants_cl100k("gpt-4-turbo"));
        assert!(!wants_cl100k("gpt-4o"));
        assert!(!wants_cl100k("gpt-4.1"));
        assert!(!wants_cl100k("gpt-5.2"));
        assert!(!wants_cl100k("o3"));
        assert!(!wants_cl100k("mystery-model"));
    }

    #[test]
    fn namespaced_model_ids_select_the_encoder_on_the_bare_id() {
        // A provider namespace (e.g. codex's `codex/gpt-5.2`) never changes
        // the estimate: encoder selection strips it before comparing families.
        let namespaced = count_chat_tokens("codex/gpt-5.2", None, &[], &[user("hi")]);
        let bare = count_chat_tokens("gpt-5.2", None, &[], &[user("hi")]);
        assert_eq!(namespaced, bare);
    }

    #[test]
    fn framing_constants_apply_per_message_plus_priming() {
        let bpe = o200k_base_singleton();
        let hello = count(bpe, "hello world");
        let one = count_chat_tokens("gpt-5", None, &[], &[user("hello world")]);
        assert_eq!(one, TOKENS_REPLY_PRIMING + TOKENS_PER_MESSAGE + hello);

        let two = count_chat_tokens(
            "gpt-5",
            None,
            &[],
            &[user("hello world"), user("hello world")],
        );
        assert_eq!(two, TOKENS_REPLY_PRIMING + 2 * (TOKENS_PER_MESSAGE + hello));
    }

    #[test]
    fn system_prompt_and_tools_count_toward_the_total() {
        let base = count_chat_tokens("gpt-5", None, &[], &[user("hi")]);
        let tools = vec![AgentFunction {
            name: "agent::trigger".into(),
            description: "Invoke an iii function".into(),
            parameters: json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }];
        let with_extras = count_chat_tokens("gpt-5", Some("be brief"), &tools, &[user("hi")]);
        assert!(with_extras > base);
    }

    #[test]
    fn function_calls_and_results_contribute_their_wire_text() {
        let assistant = AgentMessage::Assistant(AssistantMessage {
            role: AssistantRoleTag::Assistant,
            content: vec![ContentBlock::FunctionCall {
                id: "t1".into(),
                function_id: "shell::exec".into(),
                arguments: json!({ "cmd": "ls" }),
            }],
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            error_kind: None,
            warnings: None,
            usage: None,
            model: "m".into(),
            provider: "openai".into(),
            timestamp: 2,
        });
        let text = message_text(&assistant).unwrap();
        assert!(text.contains("shell__exec"));
        assert!(text.contains(r#"{"cmd":"ls"}"#));
    }
}
