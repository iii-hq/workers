//! `router::transcribe` and `router::speak` — the speech front doors, shaped
//! like `router::embed`: the router resolves a provider, forwards to
//! `provider::<id>::transcribe` or `provider::<id>::speak`, and stamps the
//! provider on the reply. Providers declare speech models with a `speech`
//! block on the model record, so a caller can name a model and the router
//! finds its owner, consoles can list them, and a chat picker never sees
//! them. A provider named explicitly is called even when its catalog slice
//! is cold; one without the surface answers function-not-found, which
//! becomes a typed `router/no_speech_provider` error.

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::catalog::queries::effective_model_ref;
use crate::catalog::store::CatalogStore;
use crate::registry::store::RegistryStore;
use crate::types::errors::{is_function_not_found, RouterCode, RouterError};
use crate::types::model::{Model, SpeechModality};

/// Recordings can run long and providers decode at a fraction of real time.
const TRANSCRIBE_TIMEOUT_MS: u64 = 300_000;
/// Synthesis of a few paragraphs; providers stream nothing back here.
const SPEAK_TIMEOUT_MS: u64 = 120_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RouterTranscribeRequest {
    /// Speech-to-text model id (an `stt` model from `router::models::list`);
    /// the provider's default when omitted.
    #[serde(default)]
    pub model: Option<String>,
    /// Provider id; resolved from the model's catalog owner when omitted.
    #[serde(default)]
    pub provider: Option<String>,
    /// The audio, base64. WAV unless `mime` says otherwise.
    pub audio_base64: String,
    /// MIME type of the audio: `audio/wav` (default), `audio/mpeg`,
    /// `audio/webm`, `audio/ogg`, `audio/flac`.
    #[serde(default)]
    pub mime: Option<String>,
    /// BCP-47 language hint; the model detects the language when omitted.
    #[serde(default)]
    pub language: Option<String>,
    /// Vocabulary or context hint for the recognizer, when the provider
    /// takes one.
    #[serde(default)]
    pub prompt: Option<String>,
}

/// One timed span of a transcript; times are absent when the provider
/// gives none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TranscriptSegment {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_secs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_secs: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RouterTranscribeResponse {
    pub provider: String,
    pub model: String,
    /// The whole transcript.
    pub text: String,
    /// Timed spans when the provider produces them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<TranscriptSegment>,
    /// Language the provider detected or was told (BCP-47).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RouterSpeakRequest {
    /// Text-to-speech model id (a `tts` model from `router::models::list`);
    /// the provider's default when omitted.
    #[serde(default)]
    pub model: Option<String>,
    /// Provider id; resolved from the model's catalog owner when omitted.
    #[serde(default)]
    pub provider: Option<String>,
    /// Text to speak.
    pub text: String,
    /// Voice id or name as the provider knows it; the provider's default
    /// when omitted.
    #[serde(default)]
    pub voice: Option<String>,
    /// Audio container wanted: `mp3` (default), `wav`, `pcm16`, `opus`.
    /// Providers answer with what they can and say so in `mime`.
    #[serde(default)]
    pub format: Option<String>,
    /// BCP-47 language hint for multilingual voices.
    #[serde(default)]
    pub language: Option<String>,
    /// Speaking-rate multiplier; 1.0 is the voice's own pace.
    #[serde(default)]
    pub speed: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RouterSpeakResponse {
    pub provider: String,
    pub model: String,
    /// The audio, base64.
    pub audio_base64: String,
    /// MIME type of the audio, e.g. `audio/mpeg`.
    pub mime: String,
    /// Voice the provider used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
}

/// What `provider::<id>::transcribe` answers; `provider` is stamped on by
/// the router.
#[derive(Debug, Deserialize)]
struct ProviderTranscribeReply {
    model: String,
    text: String,
    #[serde(default)]
    segments: Vec<TranscriptSegment>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    duration_secs: Option<f64>,
}

/// What `provider::<id>::speak` answers; `provider` is stamped on by the
/// router.
#[derive(Debug, Deserialize)]
struct ProviderSpeakReply {
    model: String,
    audio_base64: String,
    #[serde(default)]
    mime: Option<String>,
    #[serde(default)]
    voice: Option<String>,
    #[serde(default)]
    duration_secs: Option<f64>,
}

fn surface_name(modality: SpeechModality) -> &'static str {
    match modality {
        SpeechModality::Stt => "transcribe",
        SpeechModality::Tts => "speak",
    }
}

/// The provider a speech request goes to and the model id it carries
/// (`None` = the provider's default). Pure so the rules are pinned in tests:
///
/// 1. A composite `provider::model` id splits when its prefix names a
///    registered or catalog provider, as everywhere else in the router.
/// 2. An explicit provider must be registered; it gets the caller's model
///    or, with none named, its own default (`None` on the wire).
/// 3. A model without a provider resolves through the catalog: exactly one
///    owner with this modality wins, a model of the other modality is a
///    caller error, several owners are ambiguous, none is unserved.
/// 4. Neither: the first provider (by id) that declared a model of this
///    modality, which then uses its own default.
pub fn resolve_speech_target(
    modality: SpeechModality,
    model: Option<&str>,
    provider: Option<&str>,
    registered: &[String],
    catalog: &[Model],
) -> Result<(String, Option<String>), RouterError> {
    let provider = provider.map(str::trim).filter(|p| !p.is_empty());
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    let registered_has = |id: &str| registered.iter().any(|p| p == id);
    let (provider, model) = match model {
        Some(model) => {
            let (p, m) = effective_model_ref(provider, model, |p| {
                registered_has(p) || catalog.iter().any(|c| c.provider == p)
            });
            (p, Some(m))
        }
        None => (provider, None),
    };
    let of_modality = |m: &Model| m.speech_modality() == Some(modality);
    let surface = surface_name(modality);

    if let Some(provider) = provider {
        if !registered_has(provider) {
            return Err(RouterError::new(
                RouterCode::UnknownProvider,
                format!(
                    "Provider \"{provider}\" is not registered. Choose a configured provider and try again."
                ),
            ));
        }
        return Ok((provider.to_string(), model.map(str::to_string)));
    }

    if let Some(model) = model {
        let mut owners: Vec<&str> = catalog
            .iter()
            .filter(|m| m.id == model && of_modality(m))
            .map(|m| m.provider.as_str())
            .collect();
        owners.sort_unstable();
        owners.dedup();
        return match owners.as_slice() {
            [owner] => Ok((owner.to_string(), Some(model.to_string()))),
            [] => {
                if let Some(other) = catalog
                    .iter()
                    .find(|m| m.id == model && m.speech.is_some() && !of_modality(m))
                {
                    let kind = other.speech_modality().map(surface_name).unwrap_or("chat");
                    return Err(RouterError::new(
                        RouterCode::InvalidRequest,
                        format!(
                            "Model \"{model}\" is a {kind} model; router::{surface} needs a {surface} model."
                        ),
                    ));
                }
                Err(RouterError::new(
                    RouterCode::NoProviderForModel,
                    format!(
                        "No registered provider serves the {surface} model \"{model}\". \
                         router::models::list with modality \"{}\" names the choices.",
                        match modality {
                            SpeechModality::Stt => "stt",
                            SpeechModality::Tts => "tts",
                        }
                    ),
                ))
            }
            many => Err(RouterError::new(
                RouterCode::AmbiguousModel,
                format!(
                    "Model \"{model}\" is served by {}; pass provider to choose one.",
                    many.join(", ")
                ),
            )),
        };
    }

    let mut candidates: Vec<&str> = catalog
        .iter()
        .filter(|m| of_modality(m) && registered_has(&m.provider))
        .map(|m| m.provider.as_str())
        .collect();
    candidates.sort_unstable();
    match candidates.first() {
        Some(provider) => Ok((provider.to_string(), None)),
        None => Err(RouterError::new(
            RouterCode::NoProviderForModel,
            format!(
                "No registered provider has declared a {surface} model. Add a speech provider \
                 worker or pass provider explicitly."
            ),
        )),
    }
}

async fn forward(
    iii: &IIIClient,
    provider: &str,
    modality: SpeechModality,
    payload: Value,
    timeout_ms: u64,
) -> Result<Value, Error> {
    let surface = surface_name(modality);
    let function_id = format!("provider::{provider}::{surface}");
    match iii
        .trigger(TriggerRequest {
            function_id: function_id.clone(),
            payload,
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await
    {
        Ok(reply) => Ok(reply),
        Err(e) if is_function_not_found(&e) => Err(Error::Handler(format!(
            "router/no_speech_provider: provider \"{provider}\" has no {surface} surface \
             ({function_id}); choose a speech provider or install one"
        ))),
        Err(e) => Err(e),
    }
}

fn bad_reply(provider: &str, modality: SpeechModality, what: &str) -> Error {
    Error::Handler(format!(
        "router/bad_provider_response: provider::{provider}::{} {what}",
        surface_name(modality)
    ))
}

pub fn make_transcribe(
    iii: IIIClient,
    registry: Arc<RegistryStore>,
    catalog: Arc<CatalogStore>,
) -> impl Fn(RouterTranscribeRequest) -> BoxedTranscribeFuture + Send + Sync + 'static {
    move |req: RouterTranscribeRequest| {
        let (iii, registry, catalog) = (iii.clone(), registry.clone(), catalog.clone());
        Box::pin(async move {
            if req.audio_base64.trim().is_empty() {
                return Err(RouterError::new(
                    RouterCode::InvalidRequest,
                    "audio_base64 must not be empty",
                )
                .into());
            }
            let registered = registry.ids().await;
            let models = catalog.all().await;
            let (provider, model) = resolve_speech_target(
                SpeechModality::Stt,
                req.model.as_deref(),
                req.provider.as_deref(),
                &registered,
                &models,
            )?;
            let reply = forward(
                &iii,
                &provider,
                SpeechModality::Stt,
                json!({
                    "model": model,
                    "audio_base64": req.audio_base64,
                    "mime": req.mime.unwrap_or_else(|| "audio/wav".to_string()),
                    "language": req.language,
                    "prompt": req.prompt,
                }),
                TRANSCRIBE_TIMEOUT_MS,
            )
            .await?;
            let reply: ProviderTranscribeReply = serde_json::from_value(reply).map_err(|e| {
                bad_reply(
                    &provider,
                    SpeechModality::Stt,
                    &format!("returned no usable transcript: {e}"),
                )
            })?;
            if reply.model.is_empty() {
                return Err(bad_reply(
                    &provider,
                    SpeechModality::Stt,
                    "returned no model",
                ));
            }
            Ok(RouterTranscribeResponse {
                provider,
                model: reply.model,
                text: reply.text,
                segments: reply.segments,
                language: reply.language,
                duration_secs: reply.duration_secs,
            })
        })
    }
}

pub fn make_speak(
    iii: IIIClient,
    registry: Arc<RegistryStore>,
    catalog: Arc<CatalogStore>,
) -> impl Fn(RouterSpeakRequest) -> BoxedSpeakFuture + Send + Sync + 'static {
    move |req: RouterSpeakRequest| {
        let (iii, registry, catalog) = (iii.clone(), registry.clone(), catalog.clone());
        Box::pin(async move {
            if req.text.trim().is_empty() {
                return Err(
                    RouterError::new(RouterCode::InvalidRequest, "text must not be empty").into(),
                );
            }
            let registered = registry.ids().await;
            let models = catalog.all().await;
            let (provider, model) = resolve_speech_target(
                SpeechModality::Tts,
                req.model.as_deref(),
                req.provider.as_deref(),
                &registered,
                &models,
            )?;
            let format = req.format.unwrap_or_else(|| "mp3".to_string());
            let reply = forward(
                &iii,
                &provider,
                SpeechModality::Tts,
                json!({
                    "model": model,
                    "text": req.text,
                    "voice": req.voice,
                    "format": format,
                    "language": req.language,
                    "speed": req.speed,
                }),
                SPEAK_TIMEOUT_MS,
            )
            .await?;
            let reply: ProviderSpeakReply = serde_json::from_value(reply).map_err(|e| {
                bad_reply(
                    &provider,
                    SpeechModality::Tts,
                    &format!("returned no usable audio: {e}"),
                )
            })?;
            if reply.model.is_empty() {
                return Err(bad_reply(
                    &provider,
                    SpeechModality::Tts,
                    "returned no model",
                ));
            }
            if reply.audio_base64.is_empty() {
                return Err(bad_reply(
                    &provider,
                    SpeechModality::Tts,
                    "returned empty audio_base64",
                ));
            }
            Ok(RouterSpeakResponse {
                provider,
                model: reply.model,
                audio_base64: reply.audio_base64,
                mime: reply
                    .mime
                    .unwrap_or_else(|| mime_for_format(&format).to_string()),
                voice: reply.voice,
                duration_secs: reply.duration_secs,
            })
        })
    }
}

/// The MIME type a requested `format` implies when a provider answers
/// without naming one.
fn mime_for_format(format: &str) -> &'static str {
    match format {
        "wav" => "audio/wav",
        "pcm16" => "audio/pcm",
        "opus" => "audio/ogg",
        _ => "audio/mpeg",
    }
}

type BoxedTranscribeFuture =
    std::pin::Pin<Box<dyn Future<Output = Result<RouterTranscribeResponse, Error>> + Send>>;
type BoxedSpeakFuture =
    std::pin::Pin<Box<dyn Future<Output = Result<RouterSpeakResponse, Error>> + Send>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::model::SpeechModel;

    #[test]
    fn mime_fallback_follows_the_requested_format() {
        assert_eq!(mime_for_format("mp3"), "audio/mpeg");
        assert_eq!(mime_for_format("wav"), "audio/wav");
        assert_eq!(mime_for_format("pcm16"), "audio/pcm");
        assert_eq!(mime_for_format("opus"), "audio/ogg");
    }

    fn speech(id: &str, provider: &str, modality: SpeechModality) -> Model {
        Model {
            id: id.into(),
            provider: provider.into(),
            display_name: None,
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
                languages: vec!["en".into()],
                streaming: false,
            }),
        }
    }

    fn chat(id: &str, provider: &str) -> Model {
        Model {
            speech: None,
            context_window: 128_000,
            max_output_tokens: 8_192,
            ..speech(id, provider, SpeechModality::Stt)
        }
    }

    fn registered(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_model_id_resolves_to_its_single_owner() {
        let catalog = vec![
            speech("scribe", "elevenlabs", SpeechModality::Stt),
            speech("eleven-v3", "elevenlabs", SpeechModality::Tts),
            chat("gpt-x", "openai"),
        ];
        let reg = registered(&["elevenlabs", "openai"]);
        assert_eq!(
            resolve_speech_target(SpeechModality::Stt, Some("scribe"), None, &reg, &catalog),
            Ok(("elevenlabs".into(), Some("scribe".into())))
        );
        assert_eq!(
            resolve_speech_target(SpeechModality::Tts, Some("eleven-v3"), None, &reg, &catalog),
            Ok(("elevenlabs".into(), Some("eleven-v3".into())))
        );
    }

    #[test]
    fn the_wrong_modality_is_a_caller_error_and_unknown_ids_are_unserved() {
        let catalog = vec![speech("eleven-v3", "elevenlabs", SpeechModality::Tts)];
        let reg = registered(&["elevenlabs"]);
        let err =
            resolve_speech_target(SpeechModality::Stt, Some("eleven-v3"), None, &reg, &catalog)
                .unwrap_err();
        assert_eq!(err.code, RouterCode::InvalidRequest);
        let err = resolve_speech_target(SpeechModality::Stt, Some("nope"), None, &reg, &catalog)
            .unwrap_err();
        assert_eq!(err.code, RouterCode::NoProviderForModel);
    }

    #[test]
    fn two_owners_are_ambiguous_until_a_provider_is_named() {
        let catalog = vec![
            speech("whisper-1", "openai", SpeechModality::Stt),
            speech("whisper-1", "groq", SpeechModality::Stt),
        ];
        let reg = registered(&["groq", "openai"]);
        let err =
            resolve_speech_target(SpeechModality::Stt, Some("whisper-1"), None, &reg, &catalog)
                .unwrap_err();
        assert_eq!(err.code, RouterCode::AmbiguousModel);
        assert_eq!(
            resolve_speech_target(
                SpeechModality::Stt,
                Some("whisper-1"),
                Some("groq"),
                &reg,
                &catalog
            ),
            Ok(("groq".into(), Some("whisper-1".into())))
        );
        assert_eq!(
            resolve_speech_target(
                SpeechModality::Stt,
                Some("groq::whisper-1"),
                None,
                &reg,
                &catalog
            ),
            Ok(("groq".into(), Some("whisper-1".into())))
        );
    }

    #[test]
    fn an_explicit_provider_is_called_even_with_a_cold_catalog() {
        let reg = registered(&["sarvam"]);
        assert_eq!(
            resolve_speech_target(SpeechModality::Stt, None, Some("sarvam"), &reg, &[]),
            Ok(("sarvam".into(), None))
        );
        let catalog = vec![
            speech("saarika-v2", "sarvam", SpeechModality::Stt),
            speech("saarika-v1", "sarvam", SpeechModality::Stt),
            speech("bulbul-v2", "sarvam", SpeechModality::Tts),
        ];
        assert_eq!(
            resolve_speech_target(SpeechModality::Stt, None, Some("sarvam"), &reg, &catalog),
            Ok(("sarvam".into(), None))
        );
        let err = resolve_speech_target(SpeechModality::Stt, None, Some("ghost"), &reg, &catalog)
            .unwrap_err();
        assert_eq!(err.code, RouterCode::UnknownProvider);
    }

    #[test]
    fn with_nothing_named_the_first_declaring_provider_wins() {
        let catalog = vec![
            speech("chirp-3", "google-speech", SpeechModality::Stt),
            speech("scribe", "elevenlabs", SpeechModality::Stt),
            speech("eleven-v3", "elevenlabs", SpeechModality::Tts),
        ];
        let reg = registered(&["elevenlabs", "google-speech"]);
        assert_eq!(
            resolve_speech_target(SpeechModality::Stt, None, None, &reg, &catalog),
            Ok(("elevenlabs".into(), None))
        );
        let err = resolve_speech_target(
            SpeechModality::Tts,
            None,
            None,
            &registered(&["openai"]),
            &catalog,
        )
        .unwrap_err();
        assert_eq!(err.code, RouterCode::NoProviderForModel);
    }
}
