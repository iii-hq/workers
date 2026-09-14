//! `voice::speak` and `voice::speak::stop` — read text aloud.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use base64::Engine as _;

use super::AppState;
use crate::config::TtsBackend;
use crate::tts::Spoken;

pub const ID: &str = "voice::speak";
pub const DESC: &str = "Synthesize text and return audio_base64 plus mime to the requesting caller. \
                        Every backend (host, piper, openai, router) returns audio for client-side playback; \
                        nothing plays on the worker machine. host uses say/espeak; piper uses a local neural voice.";

pub const STOP_ID: &str = "voice::speak::stop";
pub const STOP_DESC: &str =
    "Legacy server-playback stop (returns stopped=0). Audio is now returned \
                            to the caller; stop its local audio element to stop browser playback.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// What to say. Capped by tts.max_speak_chars.
    pub text: String,
    /// Voice name for the backend; empty uses the configured default.
    #[serde(default)]
    pub voice: Option<String>,
    /// Speaking rate in words per minute (host backend).
    #[serde(default)]
    pub rate_wpm: Option<u32>,
}

pub type Response = Spoken;

pub async fn handle(state: &AppState, req: Request) -> Result<Response, String> {
    let cfg = state.cfg.read().await.clone();
    if cfg.tts.backend == TtsBackend::Router {
        let text = crate::tts::clip_text(&req.text, cfg.tts.max_speak_chars)?;
        let (audio, mime, _model) =
            crate::router::speak(&state.iii, &cfg, &text, req.voice.as_deref()).await?;
        return Ok(Spoken {
            backend: "router".into(),
            speech_id: format!("s_{}", uuid::Uuid::new_v4().simple()),
            played: false,
            audio_base64: Some(base64::engine::general_purpose::STANDARD.encode(audio)),
            mime: Some(mime),
        });
    }
    state
        .speaker
        .speak(&cfg, &req.text, req.voice.as_deref(), req.rate_wpm)
        .await
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct StopRequest {
    /// Legacy playback id; no effect on client-side audio.
    #[serde(default)]
    pub speech_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StopResponse {
    /// Always zero: playback is now owned by each client.
    pub stopped: usize,
}

pub async fn stop(state: &AppState, req: StopRequest) -> Result<StopResponse, String> {
    Ok(StopResponse {
        stopped: state.speaker.stop(req.speech_id.as_deref()).await,
    })
}
