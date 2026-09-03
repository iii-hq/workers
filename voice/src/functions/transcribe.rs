//! `voice::transcribe` — a whole audio file to text.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::AppState;
use crate::audio;
use crate::engine::Segment;

pub const ID: &str = "voice::transcribe";
pub const DESC: &str =
    "Transcribe a WAV file to text with timestamped segments. Pass a path on the \
                        worker's host or the file itself as base64 (up to max_audio_bytes). Any \
                        sample rate and channel count; the audio is downmixed and resampled.";

/// Path-based reads are bounded too, so a stray multi-gigabyte file cannot
/// take the worker down.
const MAX_PATH_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// Path of a WAV file on the machine running the worker.
    #[serde(default)]
    pub path: Option<String>,
    /// The WAV file itself, base64.
    #[serde(default)]
    pub audio_base64: Option<String>,
    /// Language hint (ISO 639-1), used by the remote backend.
    #[serde(default)]
    pub language: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    pub text: String,
    pub segments: Vec<Segment>,
    pub duration_secs: f32,
    pub model: String,
    /// `local` or `openai`.
    pub backend: String,
}

pub async fn handle(state: &AppState, req: Request) -> Result<Response, String> {
    let cfg = state.cfg.read().await.clone();
    let bytes = match (req.path.as_deref(), req.audio_base64.as_deref()) {
        (Some(_), Some(_)) => {
            return Err("pass either path or audio_base64, not both".to_string());
        }
        (None, None) => {
            return Err("pass a WAV file: path (on the worker's host) or audio_base64".to_string());
        }
        (Some(path), None) => read_path(path).await?,
        (None, Some(b64)) => {
            use base64::Engine;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .map_err(|e| format!("audio_base64 is not valid base64: {e}"))?;
            if decoded.len() > cfg.max_audio_bytes {
                return Err(format!(
                    "audio is {} bytes, over the {}-byte inline cap (max_audio_bytes); pass a path instead",
                    decoded.len(),
                    cfg.max_audio_bytes
                ));
            }
            decoded
        }
    };
    let decoded = tokio::task::spawn_blocking(move || audio::decode_wav(&bytes))
        .await
        .map_err(|e| format!("decode task failed: {e}"))??;
    if decoded.samples.is_empty() {
        return Err("the file holds no audio samples".to_string());
    }
    let (transcript, backend, model) = state
        .engine
        .transcribe(&cfg, decoded.samples, req.language.as_deref())
        .await?;
    Ok(Response {
        text: transcript.text,
        segments: transcript.segments,
        duration_secs: transcript.duration_secs,
        model,
        backend: backend.to_string(),
    })
}

async fn read_path(path: &str) -> Result<Vec<u8>, String> {
    let resolved = iii_worker_paths::resolve_path(path);
    let meta = tokio::fs::metadata(&resolved)
        .await
        .map_err(|e| format!("{}: {e}", resolved.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", resolved.display()));
    }
    if meta.len() > MAX_PATH_BYTES {
        return Err(format!(
            "{} is {} bytes, over the {MAX_PATH_BYTES}-byte limit",
            resolved.display(),
            meta.len()
        ));
    }
    tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("read {}: {e}", resolved.display()))
}
