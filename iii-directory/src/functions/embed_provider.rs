//! `provider::minilm::embed`: the local MiniLM that ranks
//! `directory::search_functions` candidates, served as an embed-only
//! provider for `router::embed`. It never declares itself to the router, so
//! it lists no models and no model picker shows it; the router tries it
//! only when every registered provider has no embed surface or is not
//! configured for good (no API key), and none whose embed answered before
//! is missing within 15 minutes of last being live (llm-router README,
//! embed-only providers). Such stacks
//! (Anthropic only, say) get semantic recall without a key or another
//! process; configuring a keyless provider later moves callers to its
//! vector space (memory re-embeds). Agents cannot call it: the root
//! permissions deny `provider::*`.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::search_semantic::{embedding_model_id, EmbedError, SemanticSearch, MINILM_REPOSITORY};

pub const EMBED_ID: &str = "provider::minilm::embed";
const EMBED_DESC: &str = "Batch text embeddings from the local MiniLM \
     (sentence-transformers/all-MiniLM-L6-v2, 384 dims, L2-normalized): one vector per input, \
     order preserved. Called by router::embed.";
/// The same batch ceiling provider-llamacpp takes.
const MAX_INPUTS: usize = 512;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EmbedRequest {
    /// Embedding model id: omitted, or the one this provider reports.
    #[serde(default)]
    pub model: Option<String>,
    /// Texts to embed, one vector returned per input, order preserved.
    pub input: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EmbedResponse {
    /// `sentence-transformers/all-MiniLM-L6-v2@<revision>`.
    pub model: String,
    /// One embedding per input, in input order.
    pub embeddings: Vec<Vec<f32>>,
}

pub fn register(iii: &Arc<IIIClient>, semantic: &SemanticSearch) {
    let semantic = semantic.clone();
    iii.register_function(
        EMBED_ID,
        RegisterFunction::new_async(move |request: EmbedRequest| {
            let semantic = semantic.clone();
            async move { handle(&semantic, request).await }
        })
        .description(EMBED_DESC)
        .metadata(json!({ "internal": true })),
    );
}

async fn handle(semantic: &SemanticSearch, request: EmbedRequest) -> Result<EmbedResponse, Error> {
    check(&request).map_err(Error::Handler)?;
    let embeddings = semantic.embed(request.input).await.map_err(|error| {
        Error::Handler(match error {
            // The prefix the router reads as "move on to the next embedder".
            EmbedError::Unconfigured(reason) => format!("provider/not_configured: {reason}"),
            EmbedError::Failed(reason) => format!("provider/internal: {reason}"),
        })
    })?;
    Ok(EmbedResponse {
        model: embedding_model_id(),
        embeddings,
    })
}

/// A request this provider can serve. Another model is "not configured",
/// not invalid: a caller set up for an API model then reaches the next
/// embedder instead of failing on this one.
fn check(request: &EmbedRequest) -> Result<(), String> {
    if request.input.is_empty() {
        return Err("provider/invalid_input: input must not be empty".into());
    }
    if request.input.len() > MAX_INPUTS {
        return Err(format!(
            "provider/invalid_input: at most {MAX_INPUTS} inputs per call, got {}",
            request.input.len()
        ));
    }
    match request.model.as_deref().filter(|m| !m.is_empty()) {
        None => Ok(()),
        Some(model) if model == MINILM_REPOSITORY || model == embedding_model_id() => Ok(()),
        Some(model) => Err(format!(
            "provider/not_configured: minilm serves {} only, not {model}",
            embedding_model_id()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(model: Option<&str>, inputs: usize) -> EmbedRequest {
        EmbedRequest {
            model: model.map(str::to_string),
            input: vec!["text".into(); inputs],
        }
    }

    #[test]
    fn a_batch_needs_one_to_512_inputs() {
        assert!(check(&request(None, 1)).is_ok());
        assert!(check(&request(None, MAX_INPUTS)).is_ok());
        for n in [0, MAX_INPUTS + 1] {
            let err = check(&request(None, n)).unwrap_err();
            assert!(err.starts_with("provider/invalid_input:"), "{err}");
        }
    }

    #[test]
    fn another_model_is_not_configured_so_the_router_moves_on() {
        assert!(check(&request(Some(""), 1)).is_ok());
        assert!(check(&request(Some(MINILM_REPOSITORY), 1)).is_ok());
        assert!(check(&request(Some(&embedding_model_id()), 1)).is_ok());
        let err = check(&request(Some("text-embedding-3-small"), 1)).unwrap_err();
        assert!(err.starts_with("provider/not_configured:"), "{err}");
    }

    #[test]
    fn the_reported_model_pins_the_revision() {
        let id = embedding_model_id();
        assert!(
            id.starts_with("sentence-transformers/all-MiniLM-L6-v2@"),
            "{id}"
        );
        assert_eq!(id.len(), MINILM_REPOSITORY.len() + 1 + 12);
    }

    #[tokio::test]
    async fn without_a_model_directory_it_is_not_configured() {
        let err = handle(&SemanticSearch::new(None), request(None, 1))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("provider/not_configured:"), "{err}");
    }

    #[tokio::test]
    async fn an_unprovisioned_bundle_is_not_configured() {
        let dir = tempfile::tempdir().unwrap();
        let semantic = SemanticSearch::new(Some(dir.path().to_path_buf()));
        let err = handle(&semantic, request(None, 1))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("provider/not_configured:"), "{err}");
    }

    /// Needs the pinned bundle and ONNX runtime, like the search
    /// round-trip test in `search_semantic`.
    #[cfg(minilm)]
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires III_DIRECTORY_MINILM_MODEL_PATH and the pinned ONNX runtime"]
    async fn real_minilm_embeds_close_texts_closer() {
        let Ok(path) = std::env::var("III_DIRECTORY_MINILM_MODEL_PATH") else {
            return;
        };
        let semantic = SemanticSearch::new(Some(path.into()));
        let texts = [
            "I moved to Lisbon last year",
            "Where do I live now?",
            "Fix the CI",
        ];
        let resp = handle(
            &semantic,
            EmbedRequest {
                model: None,
                input: texts.iter().map(|t| t.to_string()).collect(),
            },
        )
        .await
        .unwrap();
        assert_eq!(resp.embeddings.len(), 3);
        assert!(resp.embeddings.iter().all(|v| v.len() == 384));
        let cos = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        let e = &resp.embeddings;
        assert!(cos(&e[0], &e[1]) > cos(&e[0], &e[2]));
    }
}
