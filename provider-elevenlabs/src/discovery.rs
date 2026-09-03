//! Catalog reconcile. ElevenLabs lists its text-to-speech models (with the
//! languages each speaks) on `GET /v1/models`; the Scribe speech-to-text
//! models are not on that list, so they are declared here. The configured
//! credential gates the slice: no key, empty catalog, so pickers never show
//! unusable rows.
use crate::speech::{api_url_or_default, secret_of};
use crate::{router_client, state, PROVIDER_ID};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::model::{Model, SpeechModality, SpeechModel};
use llm_router::types::router::{RefreshModelsRequest, RefreshModelsResponse};
use serde::Deserialize;

/// The Scribe speech-to-text models. Scribe v1 handles 99 languages and
/// detects the language itself; the list stays empty rather than naming
/// all of them.
pub const STT_MODELS: [(&str, &str); 2] = [
    ("scribe_v1", "Scribe v1"),
    ("scribe_v1_experimental", "Scribe v1 (experimental)"),
];

/// Text-to-speech models to declare before the first refresh lands, so a
/// fresh install has something to pick.
const STATIC_TTS_MODELS: [(&str, &str); 3] = [
    ("eleven_multilingual_v2", "Eleven Multilingual v2"),
    ("eleven_flash_v2_5", "Eleven Flash v2.5"),
    ("eleven_v3", "Eleven v3"),
];

fn speech_model(
    id: &str,
    display_name: &str,
    modality: SpeechModality,
    languages: Vec<String>,
) -> Model {
    Model {
        id: id.into(),
        provider: PROVIDER_ID.into(),
        display_name: Some(display_name.into()),
        context_window: 0,
        max_output_tokens: 0,
        input_limit: None,
        supports_thinking: None,
        supports_xhigh: None,
        reasoning_efforts: None,
        supports_tools: None,
        supports_vision: None,
        supports_cache: None,
        supports_structured_output: None,
        thinking_budgets: None,
        pricing: None,
        speech: Some(SpeechModel {
            modality,
            languages,
            streaming: false,
        }),
    }
}

pub fn stt_models() -> Vec<Model> {
    STT_MODELS
        .iter()
        .map(|(id, name)| speech_model(id, name, SpeechModality::Stt, Vec::new()))
        .collect()
}

/// The declaration-time slice: Scribe plus the well-known voices' models.
pub fn static_models() -> Vec<Model> {
    let mut models = stt_models();
    models.extend(
        STATIC_TTS_MODELS
            .iter()
            .map(|(id, name)| speech_model(id, name, SpeechModality::Tts, Vec::new())),
    );
    models
}

#[derive(Debug, Deserialize)]
struct WireLanguage {
    language_id: String,
}

#[derive(Debug, Deserialize)]
struct WireModel {
    model_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    can_do_text_to_speech: bool,
    #[serde(default)]
    languages: Vec<WireLanguage>,
}

/// `GET /v1/models` rows to catalog records: text-to-speech models only,
/// each with the language ids it speaks.
pub fn parse_live_models(json: &serde_json::Value) -> Vec<Model> {
    let rows: Vec<WireModel> = serde_json::from_value(json.clone()).unwrap_or_default();
    let mut models: Vec<Model> = rows
        .into_iter()
        .filter(|m| m.can_do_text_to_speech)
        .map(|m| {
            let name = m.name.clone().unwrap_or_else(|| m.model_id.clone());
            let languages = m.languages.into_iter().map(|l| l.language_id).collect();
            speech_model(&m.model_id, &name, SpeechModality::Tts, languages)
        })
        .collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models
}

/// The refresh flow; returns the reconciled slice size.
pub async fn refresh_models(iii: &IIIClient, http: &reqwest::Client) -> Result<usize, Error> {
    let token = state::load_token(iii).await;
    let resolved = router_client::resolve(iii, token.as_deref()).await?;
    let credential = resolved
        .credential
        .as_ref()
        .map(secret_of)
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string);
    let Some(credential) = credential else {
        // Key removed: prune the slice so the picker reflects removal
        // instead of showing stale, unusable rows.
        router_client::reconcile(iii, vec![], token.as_deref()).await?;
        return Ok(0);
    };
    let api_url = api_url_or_default(resolved.api_url.as_deref());
    let response = http
        .get(format!("{api_url}/models"))
        .header("xi-api-key", credential)
        .send()
        .await
        .map_err(|e| Error::Handler(format!("provider/upstream: models fetch failed: {e}")))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let body = response.text().await.unwrap_or_default();
        return Err(crate::errors::upstream_status(status, &body));
    }
    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| Error::Handler(format!("provider/bad_response: models list: {e}")))?;
    let mut models = stt_models();
    models.extend(parse_live_models(&json));
    let count = models.len();
    router_client::reconcile(iii, models, token.as_deref()).await?;
    Ok(count)
}

pub fn make_refresh_models(
    iii: IIIClient,
    http: reqwest::Client,
) -> impl Fn(RefreshModelsRequest) -> BoxFuture<'static, Result<RefreshModelsResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |_req: RefreshModelsRequest| {
        let (iii, http) = (iii.clone(), http.clone());
        Box::pin(async move {
            let count = refresh_models(&iii, &http).await?;
            Ok(RefreshModelsResponse { ok: true, count })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_models_keep_tts_rows_with_languages() {
        let json = serde_json::json!([
            { "model_id": "eleven_v3", "name": "Eleven v3", "can_do_text_to_speech": true,
              "languages": [{ "language_id": "en", "name": "English" }, { "language_id": "hi", "name": "Hindi" }] },
            { "model_id": "eleven_english_sts_v2", "name": "Eleven English v2", "can_do_text_to_speech": false,
              "languages": [{ "language_id": "en", "name": "English" }] }
        ]);
        let models = parse_live_models(&json);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "eleven_v3");
        assert_eq!(models[0].speech_modality(), Some(SpeechModality::Tts));
        assert_eq!(
            models[0].speech.as_ref().unwrap().languages,
            vec!["en", "hi"]
        );
        assert_eq!(models[0].context_window, 0);
    }

    #[test]
    fn the_static_slice_covers_both_families() {
        let models = static_models();
        assert!(models
            .iter()
            .any(|m| m.id == "scribe_v1" && m.speech_modality() == Some(SpeechModality::Stt)));
        assert!(models.iter().any(|m| m.id == "eleven_multilingual_v2"
            && m.speech_modality() == Some(SpeechModality::Tts)));
        assert!(models.iter().all(|m| m.provider == PROVIDER_ID));
    }
}
