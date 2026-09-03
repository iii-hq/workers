//! `provider::groq::count_tokens` — prompt token counting for a provider that
//! serves several model families at once.
//!
//! Every other provider has one vocabulary: DeepSeek is DeepSeek, GLM is GLM.
//! Groq is an inference host, so a Llama, a GPT-OSS and a Qwen model sit side
//! by side behind one endpoint with three different tokenizers between them.
//! A single fixed vocabulary would be wrong for most of the catalog, and
//! borrowing tiktoken for all of it would be wrong quietly — the number would
//! read as authoritative while being off by whatever the vocabularies disagree
//! about.
//!
//! So the vocabulary is chosen per model, and a model no rule recognizes is
//! answered with the typed `no_token_counter` rather than a guess. That leaves
//! the caller on its own estimate, which is exactly where it would have been
//! without this function, and is the honest answer for a host that adds models
//! faster than any table can follow.
//!
//! Groq publishes no metering endpoint, so counting is local and costs
//! nothing. Exposed behind `router::count_tokens`.

use iii_sdk::errors::Error;
use llm_router::provider_scaffold::vocabulary_count::{
    count_chat_tokens, resolve, VocabularyRef, ESTIMATOR_TOKENIZER,
};
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The vocabulary a model id counts with, or `None` when nothing here
/// recognizes it.
///
/// Matching is by family rather than exact id because a host's id list turns
/// over constantly while the tokenizer behind a family does not:
/// `llama-3.1-8b-instant` and `llama-3.3-70b-versatile` share one vocabulary,
/// and so will whatever Llama variant Groq adds next.
///
/// Meta's own repositories are gated behind a licence click, which a worker
/// cannot perform, so Llama resolves through a public mirror of the identical
/// tokenizer.
fn vocabulary_for(model: &str) -> Option<VocabularyRef> {
    let id = model.to_ascii_lowercase();
    let family_is = |name: &str| id.starts_with(name) || id.contains(&format!("/{name}"));

    let repo = if family_is("gpt-oss") {
        "openai/gpt-oss-20b"
    } else if family_is("llama") {
        "NousResearch/Meta-Llama-3.1-8B-Instruct"
    } else if family_is("qwen") {
        "Qwen/Qwen3-32B"
    } else {
        return None;
    };
    Some(VocabularyRef::huggingface(repo))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets; selects which family's vocabulary counts
    /// it.
    pub model: String,
    /// System prompt counted as its own wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; each serialized schema counts toward the
    /// total.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::groq::stream` accepts.
    /// Must be non-empty.
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Prompt tokens for the assembled request, by this model's own
    /// vocabulary.
    pub tokens: u64,
    /// Always `tokenizer`: the model's own vocabulary produced the count.
    pub estimator: String,
}

pub async fn handle(req: CountTokensRequest) -> Result<CountTokensResponse, Error> {
    // Dumb pipe: an empty request is a caller bug, never padded into a
    // countable one with placeholder messages.
    if req.messages.is_empty() {
        return Err(Error::Handler(
            "invalid_input: messages must not be empty".into(),
        ));
    }
    let reference = vocabulary_for(&req.model).ok_or_else(|| {
        Error::Handler(format!(
            "router/no_token_counter: no published vocabulary is known for '{}'",
            req.model
        ))
    })?;
    let tokenizer = resolve(&reference).await.ok_or_else(|| {
        Error::Handler(format!(
            "router/no_token_counter: the vocabulary for '{}' is not cached and \
             could not be fetched",
            req.model
        ))
    })?;
    let tokens = count_chat_tokens(
        &tokenizer,
        req.system_prompt.as_deref(),
        req.tools.as_deref().unwrap_or(&[]),
        &req.messages,
    );
    Ok(CountTokensResponse {
        model: req.model,
        tokens,
        estimator: ESTIMATOR_TOKENIZER.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_for(model: &str) -> Option<String> {
        vocabulary_for(model).map(|v| v.id)
    }

    #[test]
    fn each_family_resolves_to_its_own_vocabulary() {
        let llama = repo_for("llama-3.1-8b-instant");
        let gpt_oss = repo_for("openai/gpt-oss-120b");
        let qwen = repo_for("qwen3-32b");
        assert!(llama.is_some() && gpt_oss.is_some() && qwen.is_some());
        // The point of the whole module: these must not collapse onto one.
        assert_ne!(llama, gpt_oss);
        assert_ne!(gpt_oss, qwen);
        assert_ne!(llama, qwen);
    }

    #[test]
    fn a_family_shares_one_vocabulary_across_its_sizes_and_versions() {
        assert_eq!(
            repo_for("llama-3.1-8b-instant"),
            repo_for("llama-3.3-70b-versatile")
        );
        assert_eq!(
            repo_for("openai/gpt-oss-20b"),
            repo_for("openai/gpt-oss-120b")
        );
    }

    #[test]
    fn an_unrecognized_model_gets_no_vocabulary_rather_than_a_borrowed_one() {
        // The honest answer: the caller keeps its own estimate instead of
        // being handed a confident number counted with the wrong vocabulary.
        assert_eq!(repo_for("some-model-shipped-tomorrow"), None);
        assert_eq!(repo_for("whisper-large-v3"), None);
    }

    #[test]
    fn matching_ignores_case_and_survives_a_namespace() {
        assert_eq!(
            repo_for("LLAMA-3.3-70B-VERSATILE"),
            repo_for("llama-3.3-70b-versatile")
        );
        assert_eq!(
            repo_for("meta/llama-3.3-70b"),
            repo_for("llama-3.1-8b-instant")
        );
    }
}
