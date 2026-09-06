//! The `router` engines: speech to text and text to speech through
//! llm-router's `router::transcribe` and `router::speak`, so any speech
//! provider registered with the router (ElevenLabs, OpenAI, a self-hosted
//! server) serves the voice worker without the worker knowing its API.
//! Model ids are the router's `provider::model` form or a bare id the
//! router resolves; an empty id lets the router pick.

use base64::Engine as _;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::audio;
use crate::audio::TARGET_SAMPLE_RATE;
use crate::config::WorkerConfig;
use crate::engine::{Segment, Transcript};

const TRANSCRIBE_TIMEOUT_MS: u64 = 300_000;
const SPEAK_TIMEOUT_MS: u64 = 120_000;
const LIST_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug, Deserialize)]
struct WireSegment {
    text: String,
    #[serde(default)]
    start_secs: Option<f64>,
    #[serde(default)]
    end_secs: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct WireTranscript {
    provider: String,
    model: String,
    text: String,
    #[serde(default)]
    segments: Vec<WireSegment>,
    #[serde(default)]
    duration_secs: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct WireSpeech {
    provider: String,
    model: String,
    audio_base64: String,
    #[serde(default)]
    mime: Option<String>,
}

/// One speech model the router lists.
#[derive(Debug, Clone, Deserialize)]
pub struct RouterSpeechModel {
    pub id: String,
    pub provider: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireModels {
    #[serde(default)]
    models: Vec<RouterSpeechModel>,
}

fn describe(err: &Error, function_id: &str) -> String {
    match err {
        Error::Remote { code, .. } if code.eq_ignore_ascii_case("function_not_found") => {
            format!("llm-router is not running, or is too old to offer {function_id}")
        }
        Error::Remote { code, message, .. } => format!("{function_id}: {code}: {message}"),
        other => format!("{function_id}: {other}"),
    }
}

async fn call(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
    timeout_ms: u64,
) -> Result<Value, String> {
    iii.trigger(TriggerRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await
    .map_err(|e| describe(&e, function_id))
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// Transcribe a whole buffer through `router::transcribe`. Returns the
/// transcript and the `provider::model` that produced it.
pub async fn transcribe(
    iii: &IIIClient,
    cfg: &WorkerConfig,
    samples: &[f32],
    language: Option<&str>,
) -> Result<(Transcript, String), String> {
    transcribe_within(iii, cfg, samples, language, TRANSCRIBE_TIMEOUT_MS).await
}

/// [`transcribe`] with the caller's deadline: dictation gives a sentence a
/// short one and keeps its streaming text past it.
pub async fn transcribe_within(
    iii: &IIIClient,
    cfg: &WorkerConfig,
    samples: &[f32],
    language: Option<&str>,
    timeout_ms: u64,
) -> Result<(Transcript, String), String> {
    let wav = audio::encode_wav(samples, TARGET_SAMPLE_RATE)?;
    let language = language
        .and_then(non_empty)
        .or_else(|| non_empty(&cfg.stt.router.language));
    let reply = call(
        iii,
        "router::transcribe",
        json!({
            "model": non_empty(&cfg.stt.router.model),
            "audio_base64": base64::engine::general_purpose::STANDARD.encode(wav),
            "mime": "audio/wav",
            "language": language,
        }),
        timeout_ms,
    )
    .await?;
    let wire: WireTranscript = serde_json::from_value(reply)
        .map_err(|e| format!("router::transcribe answered an unexpected shape: {e}"))?;
    let duration_secs = wire
        .duration_secs
        .map(|d| d as f32)
        .unwrap_or(samples.len() as f32 / TARGET_SAMPLE_RATE as f32);
    let segments: Vec<Segment> = if wire.segments.is_empty() {
        vec![Segment {
            segment: 0,
            text: wire.text.clone(),
            start_secs: None,
            end_secs: None,
        }]
    } else {
        wire.segments
            .into_iter()
            .enumerate()
            .map(|(i, s)| Segment {
                segment: i as u32,
                text: s.text,
                start_secs: s.start_secs.map(|v| v as f32),
                end_secs: s.end_secs.map(|v| v as f32),
            })
            .collect()
    };
    Ok((
        Transcript {
            text: wire.text,
            segments,
            duration_secs,
        },
        format!("{}::{}", wire.provider, wire.model),
    ))
}

/// Speak `text` through `router::speak`. Returns the audio bytes, their
/// MIME type and the `provider::model` that produced them.
pub async fn speak(
    iii: &IIIClient,
    cfg: &WorkerConfig,
    text: &str,
    voice: Option<&str>,
) -> Result<(Vec<u8>, String, String), String> {
    let voice = voice
        .and_then(non_empty)
        .or_else(|| non_empty(&cfg.tts.router.voice));
    let reply = call(
        iii,
        "router::speak",
        json!({
            "model": non_empty(&cfg.tts.router.model),
            "text": text,
            "voice": voice,
            "format": non_empty(&cfg.tts.router.format).unwrap_or("mp3"),
        }),
        SPEAK_TIMEOUT_MS,
    )
    .await?;
    let wire: WireSpeech = serde_json::from_value(reply)
        .map_err(|e| format!("router::speak answered an unexpected shape: {e}"))?;
    let audio = base64::engine::general_purpose::STANDARD
        .decode(wire.audio_base64.trim())
        .map_err(|e| format!("router::speak returned audio that is not base64: {e}"))?;
    Ok((
        audio,
        wire.mime.unwrap_or_else(|| "audio/mpeg".to_string()),
        format!("{}::{}", wire.provider, wire.model),
    ))
}

/// The speech models the router knows for one family (`stt` or `tts`).
pub async fn models(iii: &IIIClient, modality: &str) -> Result<Vec<RouterSpeechModel>, String> {
    let reply = call(
        iii,
        "router::models::list",
        json!({ "modality": modality }),
        LIST_TIMEOUT_MS,
    )
    .await?;
    let wire: WireModels = serde_json::from_value(reply)
        .map_err(|e| format!("router::models::list answered an unexpected shape: {e}"))?;
    Ok(wire.models)
}

/// What stands in the way of the router engine, in one sentence, or `None`.
pub async fn problem(iii: &IIIClient, modality: &str) -> Option<String> {
    match models(iii, modality).await {
        Ok(list) if list.is_empty() => Some(format!(
            "llm-router lists no {} models; install a speech provider (for example \
             provider-elevenlabs) and add its key in Settings",
            if modality == "stt" {
                "speech-to-text"
            } else {
                "text-to-speech"
            }
        )),
        Ok(_) => None,
        Err(e) => Some(e),
    }
}
