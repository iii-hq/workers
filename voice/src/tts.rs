//! Read-aloud synthesis. Every backend returns audio to the requesting caller.
//! `host` runs say/espeak in file-output mode: it never opens a server speaker.

use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use tokio::process::Command;

use crate::config::{PiperDevice, TtsBackend, WorkerConfig};
use crate::events::Emitter;

const SYNTHESIS_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_SYNTHESIS_BYTES: u64 = 64 * 1024 * 1024;

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

/// Voice/rate arguments. Synthesis MUST also add file-output arguments below.
/// Text goes through stdin without a shell.
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

/// Both commands must write a private WAV, never their default audio device.
fn file_output_args(kind: HostKind, output: &std::path::Path) -> Vec<std::ffi::OsString> {
    match kind {
        HostKind::Say => vec![
            "--file-format=WAVE".into(),
            "--data-format=LEI16@22050".into(),
            "-o".into(),
            output.as_os_str().to_owned(),
        ],
        HostKind::Espeak => vec!["-w".into(), output.as_os_str().to_owned()],
    }
}

async fn local_speech(
    command: &HostCommand,
    text: &str,
    voice: &str,
    rate: u32,
) -> Result<Vec<u8>, String> {
    synthesize_wav(command.program, text, |path| {
        let mut args = file_output_args(command.kind, path);
        args.extend(
            host_args(command.kind, voice, rate)
                .into_iter()
                .map(Into::into),
        );
        args
    })
    .await
}

/// Resolve both executable and installed weights without silently falling back to eSpeak.
pub fn piper_paths(cfg: &WorkerConfig) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let command = which::which(&cfg.tts.piper.command)
        .map_err(|_| "Piper is not installed; install piper-tts and set tts.piper.command to its executable path".to_string())?;
    let model = crate::models::find(&cfg.tts.piper.model)
        .filter(|m| m.kind == crate::models::ModelKind::PiperOnnx)
        .ok_or("Choose a Piper voice from Voice → Models")?;
    if !model.is_installed(&cfg.models_path()) {
        return Err(format!(
            "Piper voice {} is missing; download it in Voice → Models",
            model.id
        ));
    }
    Ok((
        command,
        model.dir(&cfg.models_path()).join(model.files[0].name),
    ))
}

fn piper_args(
    model: &std::path::Path,
    output: &std::path::Path,
    cuda: bool,
) -> Vec<std::ffi::OsString> {
    let mut args = vec![
        "--model".into(),
        model.as_os_str().to_owned(),
        "--output-file".into(),
        output.as_os_str().to_owned(),
    ];
    if cuda {
        args.push("--cuda".into());
    }
    args
}

/// At most two attempts. In auto mode each gets half the existing budget so
/// fallback cannot double the maximum synthesis time or outlive the UI request.
async fn piper_with_fallback<T, F, Fut>(device: PiperDevice, mut synthesize: F) -> Result<T, String>
where
    F: FnMut(bool, Duration) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    if device == PiperDevice::Cpu {
        return synthesize(false, SYNTHESIS_TIMEOUT).await;
    }
    match synthesize(true, SYNTHESIS_TIMEOUT / 2).await {
        Ok(audio) => Ok(audio), // ONNX Runtime may itself have fallen back to CPU.
        Err(cuda_error) => {
            tracing::warn!("Piper CUDA-preferred attempt failed; retrying once on CPU");
            synthesize(false, SYNTHESIS_TIMEOUT / 2).await.map_err(|cpu_error|
                format!("Piper automatic synthesis failed; CUDA-preferred attempt: {cuda_error}; CPU attempt: {cpu_error}"))
        }
    }
}

async fn synthesize_wav(
    program: impl AsRef<std::ffi::OsStr>,
    text: &str,
    args: impl FnOnce(&std::path::Path) -> Vec<std::ffi::OsString>,
) -> Result<Vec<u8>, String> {
    synthesize_wav_with_timeout(program, text, args, SYNTHESIS_TIMEOUT).await
}

async fn synthesize_wav_with_timeout(
    program: impl AsRef<std::ffi::OsStr>,
    text: &str,
    args: impl FnOnce(&std::path::Path) -> Vec<std::ffi::OsString>,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    // A fresh directory per request isolates concurrent clients and is removed
    // on success, error, timeout or cancellation.
    let dir = tempfile::tempdir().map_err(|e| format!("create speech directory: {e}"))?;
    let path = dir.path().join("speech.wav");
    let mut child = Command::new(program)
        .args(args(&path))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawn speech synthesizer: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("speech command has no stdin")?;
    let operation = async {
        use tokio::io::AsyncWriteExt;
        let write = async move {
            stdin.write_all(text.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.shutdown().await
        };
        // Drain diagnostics while writing so neither pipe can block the other.
        let (_, output) = tokio::try_join!(write, child.wait_with_output())
            .map_err(|e| format!("synthesize speech: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "speech synthesizer failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(300)
                    .collect::<String>()
            ));
        }
        let size = tokio::fs::metadata(&path)
            .await
            .map_err(|e| format!("speech output: {e}"))?
            .len();
        if size > MAX_SYNTHESIS_BYTES {
            return Err("synthesized speech exceeds the 64 MiB audio limit".to_string());
        }
        let audio = tokio::fs::read(&path)
            .await
            .map_err(|e| format!("read speech WAV: {e}"))?;
        let wav = hound::WavReader::new(std::io::Cursor::new(&audio))
            .map_err(|e| format!("invalid speech WAV: {e}"))?;
        if wav.len() == 0 {
            return Err("speech command returned an empty WAV".to_string());
        }
        Ok(audio)
    };
    tokio::time::timeout(timeout, operation)
        .await
        .map_err(|_| {
            format!(
                "local speech synthesis timed out after {} seconds",
                timeout.as_secs()
            )
        })?
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
    /// `host`, `piper`, `openai`, or `router`.
    pub backend: String,
    /// Unique id of this synthesis result.
    pub speech_id: String,
    /// Legacy compatibility field; always false. Playback belongs to the caller.
    pub played: bool,
    /// Base64 audio for the requesting caller to play, on every enabled backend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_base64: Option<String>,
    /// MIME type of `audio_base64`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

pub struct Speaker;

impl Speaker {
    pub fn new(_emitter: Arc<Emitter>) -> Self {
        Self
    }

    /// Synthesize `text` on the configured backend; never play it on the server.
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
                let audio = local_speech(&command, &text, voice, rate).await?;
                Ok(Spoken {
                    backend: "host".into(),
                    speech_id,
                    played: false,
                    audio_base64: Some(BASE64_STANDARD.encode(audio)),
                    mime: Some("audio/wav".into()),
                })
            }
            TtsBackend::Piper => {
                let (command, model) = piper_paths(cfg)?;
                let audio = piper_with_fallback(cfg.tts.piper.device, |cuda, timeout| {
                    let command = &command;
                    let model = &model;
                    let text = &text;
                    async move {
                        synthesize_wav_with_timeout(
                            command,
                            text,
                            |path| piper_args(model, path, cuda),
                            timeout,
                        )
                        .await
                    }
                })
                .await?;
                Ok(Spoken {
                    backend: "piper".into(),
                    speech_id,
                    played: false,
                    audio_base64: Some(BASE64_STANDARD.encode(audio)),
                    mime: Some("audio/wav".into()),
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
            TtsBackend::Router => {
                Err("the router engine is served by voice::speak, not the host speaker".to_string())
            }
        }
    }

    /// Kept for API compatibility. There is no server playback to stop;
    /// browsers stop their own audio elements without affecting other clients.
    pub async fn stop(&self, _speech_id: Option<&str>) -> usize {
        0
    }

    /// Browser playback is client-local, not tracked on the server.
    pub async fn playing(&self) -> usize {
        0
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
        .redirect(reqwest::redirect::Policy::none())
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
    let api_key = remote.api_key.trim();
    if !api_key.is_empty() {
        crate::config::check_bearer_transport(&remote.base_url, api_key)?;
        request = request.bearer_auth(api_key);
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
    use crate::events::{NoopDeliverer, TriggerSets};

    fn test_emitter() -> Arc<Emitter> {
        Arc::new(Emitter::new(TriggerSets::new(), Arc::new(NoopDeliverer)))
    }

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
    fn both_host_commands_require_file_output() {
        let path = std::path::Path::new("/private/client speech.wav");
        assert_eq!(
            file_output_args(HostKind::Espeak, path),
            vec![std::ffi::OsString::from("-w"), path.as_os_str().to_owned()]
        );
        assert_eq!(
            file_output_args(HostKind::Say, path),
            vec![
                std::ffi::OsString::from("--file-format=WAVE"),
                "--data-format=LEI16@22050".into(),
                "-o".into(),
                path.as_os_str().to_owned()
            ]
        );
    }

    #[tokio::test]
    #[ignore = "requires say or espeak-ng installed; generates WAV without speaker playback"]
    async fn local_tts_returns_isolated_wav_audio() {
        let speaker = Speaker::new(test_emitter());
        let mut cfg = WorkerConfig::default();
        cfg.tts.backend = TtsBackend::Host;
        let (first, second) = tokio::join!(
            speaker.speak(&cfg, "Hello from the first browser.", None, None),
            speaker.speak(
                &cfg,
                "A different reply for the second browser.",
                None,
                None
            ),
        );
        let first = first.unwrap();
        let second = second.unwrap();
        assert_ne!(first.speech_id, second.speech_id);
        assert_ne!(first.audio_base64, second.audio_base64);
        for result in [first, second] {
            assert!(!result.played);
            assert_eq!(result.mime.as_deref(), Some("audio/wav"));
            let bytes = BASE64_STANDARD
                .decode(result.audio_base64.unwrap())
                .unwrap();
            let reader = hound::WavReader::new(std::io::Cursor::new(bytes)).unwrap();
            assert!(reader.len() > 0);
        }
        assert_eq!(speaker.playing().await, 0);
        assert_eq!(speaker.stop(None).await, 0);
    }

    #[test]
    fn piper_gpu_flag_preserves_model_and_output_paths() {
        let model = std::path::Path::new("/models/voice with spaces.onnx");
        let output = std::path::Path::new("/private/result.wav");
        let cpu = piper_args(model, output, false);
        assert_eq!(
            cpu,
            vec![
                std::ffi::OsString::from("--model"),
                model.as_os_str().to_owned(),
                "--output-file".into(),
                output.as_os_str().to_owned()
            ]
        );
        let cuda = piper_args(model, output, true);
        assert_eq!(&cuda[..cpu.len()], cpu.as_slice());
        assert_eq!(cuda.last().unwrap(), "--cuda");
    }

    #[tokio::test]
    async fn automatic_device_prefers_cuda_without_duplicate_synthesis() {
        let mut calls = Vec::new();
        let result = piper_with_fallback(PiperDevice::Auto, |cuda, timeout| {
            calls.push((cuda, timeout));
            std::future::ready(Ok(vec![1u8]))
        })
        .await
        .unwrap();
        assert_eq!(result, vec![1]);
        assert_eq!(calls, vec![(true, Duration::from_secs(30))]);
    }

    #[tokio::test]
    async fn automatic_device_retries_once_on_cpu_for_cuda_errors() {
        let mut calls = Vec::new();
        let result = piper_with_fallback(PiperDevice::Auto, |cuda, timeout| {
            calls.push((cuda, timeout));
            std::future::ready(if cuda {
                Err("CUDA unavailable".into())
            } else {
                Ok(vec![2u8])
            })
        })
        .await
        .unwrap();
        assert_eq!(result, vec![2]);
        assert_eq!(
            calls,
            vec![
                (true, Duration::from_secs(30)),
                (false, Duration::from_secs(30))
            ]
        );
    }

    #[tokio::test]
    async fn cpu_mode_never_requests_cuda() {
        let mut calls = Vec::new();
        let result = piper_with_fallback(PiperDevice::Cpu, |cuda, timeout| {
            calls.push((cuda, timeout));
            std::future::ready(Ok(vec![3u8]))
        })
        .await
        .unwrap();
        assert_eq!(result, vec![3]);
        assert_eq!(calls, vec![(false, SYNTHESIS_TIMEOUT)]);
    }

    #[tokio::test]
    async fn cpu_failure_is_reported_without_another_attempt() {
        let mut calls = 0;
        let result = piper_with_fallback::<(), _, _>(PiperDevice::Cpu, |_, _| {
            calls += 1;
            std::future::ready(Err("invalid model".into()))
        })
        .await;
        assert_eq!(result.unwrap_err(), "invalid model");
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn both_failures_are_reported_and_never_loop() {
        let mut calls = 0;
        let result = piper_with_fallback::<(), _, _>(PiperDevice::Auto, |cuda, _| {
            calls += 1;
            std::future::ready(Err(if cuda { "CUDA error" } else { "CPU error" }.into()))
        })
        .await
        .unwrap_err();
        assert_eq!(calls, 2);
        assert!(result.contains("CUDA error") && result.contains("CPU error"));
    }

    #[test]
    fn piper_reports_missing_executable_without_robotic_fallback() {
        let mut cfg = WorkerConfig::default();
        cfg.tts.piper.command = "/missing/piper".into();
        assert!(piper_paths(&cfg)
            .unwrap_err()
            .contains("Piper is not installed"));
    }

    #[test]
    fn piper_requires_a_complete_catalog_voice() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = WorkerConfig {
            models_dir: dir.path().to_string_lossy().into_owned(),
            ..WorkerConfig::default()
        };
        cfg.tts.piper.command = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(piper_paths(&cfg).unwrap_err().contains("download it"));
        cfg.tts.piper.model = "whisper-tiny".into();
        assert!(piper_paths(&cfg)
            .unwrap_err()
            .contains("Choose a Piper voice"));
    }

    #[test]
    fn text_is_clipped_to_the_cap() {
        assert_eq!(clip_text("  hi  ", 10).unwrap(), "hi");
        assert!(clip_text("   ", 10).is_err());
        assert!(clip_text("too long here", 5).unwrap_err().contains("cap"));
    }

    #[tokio::test]
    async fn stop_with_nothing_playing_is_zero() {
        let speaker = Speaker::new(test_emitter());
        assert_eq!(speaker.stop(None).await, 0);
        assert_eq!(speaker.playing().await, 0);
    }

    #[tokio::test]
    async fn off_backend_refuses() {
        let mut cfg = WorkerConfig::default();
        cfg.tts.backend = TtsBackend::Off;
        let err = Speaker::new(test_emitter())
            .speak(&cfg, "hello", None, None)
            .await
            .unwrap_err();
        assert!(err.contains("disabled"));
    }
}
