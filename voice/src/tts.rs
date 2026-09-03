//! Read-aloud. Two backends, neither linked into the binary:
//!
//! - `host`: the machine's own speech command (`say` on macOS, `espeak-ng` or
//!   `espeak` on Linux) as a child process. Audio plays on the worker's host.
//! - `openai`: an OpenAI-compatible `/v1/audio/speech` endpoint. The audio
//!   comes back to the caller, base64, for playback wherever the caller is.
//!
//! Child processes are tracked so `voice::speak::stop` can end them.

use std::collections::HashMap;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::config::{TtsBackend, WorkerConfig};

/// The host command this platform speaks with, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCommand {
    pub program: &'static str,
    pub kind: HostKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    Say,
    Espeak,
}

/// Find a usable speech command on `PATH`.
pub fn host_command() -> Option<HostCommand> {
    let candidates: &[(&str, HostKind)] = if cfg!(target_os = "macos") {
        &[("say", HostKind::Say)]
    } else {
        &[
            ("espeak-ng", HostKind::Espeak),
            ("espeak", HostKind::Espeak),
        ]
    };
    candidates
        .iter()
        .find(|(program, _)| on_path(program))
        .map(|(program, kind)| HostCommand {
            program,
            kind: *kind,
        })
}

fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(program).is_file())
}

/// Arguments for the host command. Text goes through stdin so no shell and
/// no argument-length limit is involved.
pub fn host_args(kind: HostKind, voice: &str, rate_wpm: u32) -> Vec<String> {
    let mut args = Vec::new();
    match kind {
        HostKind::Say => {
            if !voice.trim().is_empty() {
                args.push("-v".into());
                args.push(voice.trim().to_string());
            }
            if rate_wpm > 0 {
                args.push("-r".into());
                args.push(rate_wpm.to_string());
            }
            args.push("-f".into());
            args.push("-".into());
        }
        HostKind::Espeak => {
            if !voice.trim().is_empty() {
                args.push("-v".into());
                args.push(voice.trim().to_string());
            }
            if rate_wpm > 0 {
                args.push("-s".into());
                args.push(rate_wpm.to_string());
            }
            args.push("--stdin".into());
        }
    }
    args
}

/// Trim a request to what one call may read.
pub fn clip_text(text: &str, max_chars: usize) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("text is empty".to_string());
    }
    let count = trimmed.chars().count();
    if count > max_chars {
        return Err(format!(
            "text is {count} characters, over the {max_chars}-character cap (tts.max_speak_chars)"
        ));
    }
    Ok(trimmed.to_string())
}

/// What a speak call produced.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct Spoken {
    /// `host` or `openai`.
    pub backend: String,
    /// Id of the playback, for `voice::speak::stop`.
    pub speech_id: String,
    /// `true` when audio started playing on the worker's host.
    pub played: bool,
    /// Base64 audio for the caller to play (openai backend only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_base64: Option<String>,
    /// MIME type of `audio_base64`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

pub struct Speaker {
    children: Mutex<HashMap<String, Child>>,
}

impl Default for Speaker {
    fn default() -> Self {
        Self::new()
    }
}

impl Speaker {
    pub fn new() -> Self {
        Self {
            children: Mutex::new(HashMap::new()),
        }
    }

    /// Speak `text` on the configured backend.
    pub async fn speak(
        &self,
        cfg: &WorkerConfig,
        text: &str,
        voice: Option<&str>,
        rate_wpm: Option<u32>,
    ) -> Result<Spoken, String> {
        let text = clip_text(text, cfg.tts.max_speak_chars)?;
        let speech_id = format!("s_{}", uuid::Uuid::new_v4().simple());
        match cfg.tts.backend {
            TtsBackend::Off => Err("read-aloud is disabled (tts.backend is off)".to_string()),
            TtsBackend::Host => {
                let command = host_command().ok_or_else(|| {
                    if cfg!(target_os = "macos") {
                        "no `say` command on PATH".to_string()
                    } else {
                        "no `espeak-ng` or `espeak` command on PATH; install one or set tts.backend to openai".to_string()
                    }
                })?;
                let voice = voice.unwrap_or(&cfg.tts.voice);
                let rate = rate_wpm.unwrap_or(cfg.tts.rate_wpm);
                let mut child = Command::new(command.program)
                    .args(host_args(command.kind, voice, rate))
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|e| format!("spawn {}: {e}", command.program))?;
                if let Some(mut stdin) = child.stdin.take() {
                    use tokio::io::AsyncWriteExt;
                    let mut body = text.clone().into_bytes();
                    body.push(b'\n');
                    stdin
                        .write_all(&body)
                        .await
                        .map_err(|e| format!("write to {}: {e}", command.program))?;
                    stdin.shutdown().await.ok();
                }
                self.reap().await;
                self.children.lock().await.insert(speech_id.clone(), child);
                Ok(Spoken {
                    backend: "host".into(),
                    speech_id,
                    played: true,
                    audio_base64: None,
                    mime: None,
                })
            }
            TtsBackend::Openai => {
                let (audio, mime) = remote_speech(cfg, &text, voice).await?;
                Ok(Spoken {
                    backend: "openai".into(),
                    speech_id,
                    played: false,
                    audio_base64: Some(BASE64_STANDARD.encode(audio)),
                    mime: Some(mime),
                })
            }
        }
    }

    /// Stop every host playback (or one, by id). Returns how many were live.
    pub async fn stop(&self, speech_id: Option<&str>) -> usize {
        let mut children = self.children.lock().await;
        let ids: Vec<String> = match speech_id {
            Some(id) => children
                .contains_key(id)
                .then(|| id.to_string())
                .into_iter()
                .collect(),
            None => children.keys().cloned().collect(),
        };
        let mut stopped = 0;
        for id in ids {
            if let Some(mut child) = children.remove(&id) {
                if child.try_wait().ok().flatten().is_none() {
                    let _ = child.kill().await;
                    stopped += 1;
                }
            }
        }
        stopped
    }

    /// `true` while any host playback is still running.
    pub async fn playing(&self) -> usize {
        self.reap().await;
        self.children.lock().await.len()
    }

    async fn reap(&self) {
        let mut children = self.children.lock().await;
        let done: Vec<String> = children
            .iter_mut()
            .filter_map(|(id, child)| child.try_wait().ok().flatten().map(|_| id.clone()))
            .collect();
        for id in done {
            children.remove(&id);
        }
    }
}

/// `POST {base_url}/audio/speech`, mp3 back.
async fn remote_speech(
    cfg: &WorkerConfig,
    text: &str,
    voice: Option<&str>,
) -> Result<(Vec<u8>, String), String> {
    let remote = &cfg.tts.openai;
    let url = format!("{}/audio/speech", remote.base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .user_agent(concat!("iii-voice/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let voice = voice
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(remote.voice.as_str());
    let body = serde_json::json!({
        "model": remote.model,
        "input": text,
        "voice": voice,
        "response_format": "mp3",
    });
    let mut request = client.post(&url).json(&body);
    if !remote.api_key.trim().is_empty() {
        request = request.bearer_auth(remote.api_key.trim());
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;
    let status = response.status();
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/mpeg")
        .split(';')
        .next()
        .unwrap_or("audio/mpeg")
        .to_string();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("read {url}: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "{url} answered {status}: {}",
            String::from_utf8_lossy(&bytes)
                .chars()
                .take(300)
                .collect::<String>()
        ));
    }
    Ok((bytes.to_vec(), mime))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_args_follow_each_command() {
        assert_eq!(
            host_args(HostKind::Say, "Samantha", 180),
            vec!["-v", "Samantha", "-r", "180", "-f", "-"]
        );
        assert_eq!(host_args(HostKind::Say, "", 0), vec!["-f", "-"]);
        assert_eq!(
            host_args(HostKind::Espeak, "en-us", 160),
            vec!["-v", "en-us", "-s", "160", "--stdin"]
        );
    }

    #[test]
    fn text_is_clipped_to_the_cap() {
        assert_eq!(clip_text("  hi  ", 10).unwrap(), "hi");
        assert!(clip_text("   ", 10).is_err());
        assert!(clip_text("too long here", 5).unwrap_err().contains("cap"));
    }

    #[tokio::test]
    async fn stop_with_nothing_playing_is_zero() {
        let speaker = Speaker::new();
        assert_eq!(speaker.stop(None).await, 0);
        assert_eq!(speaker.playing().await, 0);
    }

    #[tokio::test]
    async fn off_backend_refuses() {
        let mut cfg = WorkerConfig::default();
        cfg.tts.backend = TtsBackend::Off;
        let err = Speaker::new()
            .speak(&cfg, "hello", None, None)
            .await
            .unwrap_err();
        assert!(err.contains("disabled"));
    }
}
