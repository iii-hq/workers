//! `voice::speak` and `voice::speak::stop` — read text aloud.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::AppState;
use crate::tts::Spoken;

pub const ID: &str = "voice::speak";
pub const DESC: &str = "Read text aloud. On the host backend playback starts on the machine running \
                        the worker and the call returns at once with a speech_id; on the openai backend \
                        the audio comes back base64 for the caller to play.";

pub const STOP_ID: &str = "voice::speak::stop";
pub const STOP_DESC: &str =
    "Stop host playback: one speech_id, or every playback this worker started \
                             when none is given.";

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
    state
        .speaker
        .speak(&cfg, &req.text, req.voice.as_deref(), req.rate_wpm)
        .await
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct StopRequest {
    /// A specific playback; omit to stop them all.
    #[serde(default)]
    pub speech_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StopResponse {
    /// How many playbacks were still running and got stopped.
    pub stopped: usize,
}

pub async fn stop(state: &AppState, req: StopRequest) -> Result<StopResponse, String> {
    Ok(StopResponse {
        stopped: state.speaker.stop(req.speech_id.as_deref()).await,
    })
}
