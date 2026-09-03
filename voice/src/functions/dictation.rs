//! `voice::dictation::*` — a live microphone session: start, push audio,
//! stop, list.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::AppState;
use crate::audio::TARGET_SAMPLE_RATE;
use crate::engine::Segment;
use crate::session::SessionSummary;

pub const START_ID: &str = "voice::dictation::start";
pub const START_DESC: &str = "Open a dictation session. Transcript events (partial, final, closed) are \
                              delivered to output_function_id as audio arrives through \
                              voice::dictation::push; stop with voice::dictation::stop. Audio must be \
                              16 kHz mono 16-bit PCM.";

pub const PUSH_ID: &str = "voice::dictation::push";
pub const PUSH_DESC: &str = "Feed one chunk of 16 kHz mono 16-bit PCM (base64, at most 64 KiB) to an \
                             open dictation session. Chunks carry a rising seq; ~100 ms per chunk keeps \
                             partial text prompt.";

pub const STOP_ID: &str = "voice::dictation::stop";
pub const STOP_DESC: &str = "Close a dictation session, flush the recognizer and return the whole \
                             transcript. discard=true throws the text away instead.";

pub const LIST_ID: &str = "voice::dictation::list";
pub const LIST_DESC: &str =
    "List the dictation sessions this worker currently holds open, with their \
                             age, audio duration and idle time.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StartRequest {
    /// Function that receives every transcript event of this session.
    pub output_function_id: String,
    /// Sample rate of the audio the caller will push. Only 16000 is accepted.
    #[serde(default)]
    pub sample_rate: Option<u32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StartResponse {
    pub session_id: String,
    pub model: String,
    pub sample_rate: u32,
}

pub async fn start(state: &AppState, req: StartRequest) -> Result<StartResponse, String> {
    if let Some(rate) = req.sample_rate {
        if rate != TARGET_SAMPLE_RATE {
            return Err(format!(
                "sample_rate {rate} is not supported; resample to {TARGET_SAMPLE_RATE} Hz before pushing"
            ));
        }
    }
    let handle = state.sessions.start(req.output_function_id).await?;
    let session = handle.lock().await;
    Ok(StartResponse {
        session_id: session.id.clone(),
        model: session.model.clone(),
        sample_rate: TARGET_SAMPLE_RATE,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PushRequest {
    pub session_id: String,
    /// Rising per-session counter; a chunk not after the last accepted one
    /// is rejected.
    pub seq: u64,
    /// Little-endian signed 16-bit PCM, 16 kHz mono, base64.
    pub pcm16_base64: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PushResponse {
    pub accepted: bool,
    pub seq: u64,
    /// Milliseconds of audio this chunk held.
    pub queued_ms: u32,
}

pub async fn push(state: &AppState, req: PushRequest) -> Result<PushResponse, String> {
    let queued_ms = state
        .sessions
        .push(&req.session_id, req.seq, &req.pcm16_base64)
        .await?;
    Ok(PushResponse {
        accepted: true,
        seq: req.seq,
        queued_ms,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StopRequest {
    pub session_id: String,
    /// Drop the transcript instead of returning it.
    #[serde(default)]
    pub discard: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StopResponse {
    pub session_id: String,
    pub text: String,
    pub segments: Vec<Segment>,
    pub duration_secs: f32,
}

pub async fn stop(state: &AppState, req: StopRequest) -> Result<StopResponse, String> {
    let reason = if req.discard { "discarded" } else { "stopped" };
    let transcript = state
        .sessions
        .stop(&req.session_id, req.discard, reason)
        .await?;
    Ok(StopResponse {
        session_id: req.session_id,
        text: transcript.text,
        segments: transcript.segments,
        duration_secs: transcript.duration_secs,
    })
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListRequest {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResponse {
    pub sessions: Vec<SessionSummary>,
}

pub async fn list(state: &AppState, _req: ListRequest) -> Result<ListResponse, String> {
    Ok(ListResponse {
        sessions: state.sessions.list().await,
    })
}
