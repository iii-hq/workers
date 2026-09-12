//! Direct `whisper-cli` speech-to-text backend.
//!
//! The worker writes its normalized 16 kHz mono samples to a temporary WAV,
//! invokes whisper.cpp without a shell, and parses the CLI's JSON segments.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::audio::{self, TARGET_SAMPLE_RATE};
use crate::config::WorkerConfig;
use crate::engine::{join_segments, Segment, Transcript};

const STDERR_LIMIT: usize = 600;

/// Resolve the configured command exactly as process spawning will: explicit
/// paths use the worker path resolver, while bare names are searched on PATH.
pub fn command_path(command: &str) -> Option<PathBuf> {
    let command = command.trim();
    if command.is_empty() {
        return None;
    }
    let path = Path::new(command);
    if path.is_absolute() || path.components().count() > 1 {
        return which::which(iii_worker_paths::resolve_path(command)).ok();
    }
    which::which(command).ok()
}

/// Resolve the configured model relative to the Compose project when needed.
pub fn model_path(cfg: &WorkerConfig) -> PathBuf {
    iii_worker_paths::resolve_path(cfg.stt.whisper_cpp.model.trim())
}

/// Explain why this backend cannot run, if a required host artifact is absent.
pub fn problem(cfg: &WorkerConfig) -> Option<String> {
    let command = cfg.stt.whisper_cpp.command.trim();
    if command_path(command).is_none() {
        return Some(format!(
            "whisper.cpp command `{command}` was not found; set stt.whisper_cpp.command to whisper-cli or its absolute path"
        ));
    }
    let model = model_path(cfg);
    if !model.is_file() {
        return Some(format!(
            "whisper.cpp model `{}` was not found; set stt.whisper_cpp.model to a multilingual ggml model",
            model.display()
        ));
    }
    None
}

/// Transcribe normalized samples with the configured `whisper-cli` process.
pub async fn transcribe(
    cfg: &WorkerConfig,
    samples: &[f32],
    language: Option<&str>,
) -> Result<Transcript, String> {
    let command_name = cfg.stt.whisper_cpp.command.trim();
    let command = command_path(command_name).ok_or_else(|| {
        format!(
            "whisper.cpp command `{command_name}` was not found; set stt.whisper_cpp.command to whisper-cli or its absolute path"
        )
    })?;
    let model = model_path(cfg);
    if !model.is_file() {
        return Err(format!(
            "whisper.cpp model `{}` was not found; set stt.whisper_cpp.model to a multilingual ggml model",
            model.display()
        ));
    }

    let wav = audio::encode_wav(samples, TARGET_SAMPLE_RATE)?;
    let duration_secs = samples.len() as f32 / TARGET_SAMPLE_RATE as f32;
    let temp = tempfile::Builder::new()
        .prefix("iii-voice-whisper-")
        .tempdir()
        .map_err(|e| format!("create whisper.cpp temporary directory: {e}"))?;
    let input = temp.path().join("audio.wav");
    let output_prefix = temp.path().join("transcript");
    tokio::fs::write(&input, wav)
        .await
        .map_err(|e| format!("write whisper.cpp input: {e}"))?;

    let language = language
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let configured = cfg.stt.whisper_cpp.language.trim();
            (!configured.is_empty()).then_some(configured)
        })
        .unwrap_or("auto");
    let mut process = Command::new(&command);
    process
        .arg("--model")
        .arg(&model)
        .arg("--file")
        .arg(&input)
        .arg("--output-json")
        .arg("--output-file")
        .arg(&output_prefix)
        .arg("--no-prints")
        .arg("--threads")
        .arg(cfg.stt.num_threads.to_string())
        .arg("--language")
        .arg(language)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let output = tokio::time::timeout(
        Duration::from_secs(cfg.stt.whisper_cpp.timeout_secs),
        process.output(),
    )
    .await
    .map_err(|_| {
        format!(
            "whisper.cpp timed out after {} seconds",
            cfg.stt.whisper_cpp.timeout_secs
        )
    })?
    .map_err(|e| format!("start {}: {e}", command.display()))?;
    if !output.status.success() {
        return Err(format!(
            "whisper.cpp exited with {}: {}",
            output.status,
            clipped_stderr(&output.stderr)
        ));
    }

    let output_path = output_prefix.with_extension("json");
    let body = tokio::fs::read_to_string(&output_path)
        .await
        .map_err(|e| format!("read whisper.cpp output {}: {e}", output_path.display()))?;
    parse_output(&body, duration_secs)
}

fn clipped_stderr(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .trim()
        .chars()
        .take(STDERR_LIMIT)
        .collect()
}

fn parse_output(body: &str, duration_secs: f32) -> Result<Transcript, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("whisper.cpp returned invalid JSON: {e}"))?;
    let items = value
        .get("transcription")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "whisper.cpp output has no `transcription` array".to_string())?;
    let mut segments = Vec::with_capacity(items.len());
    for item in items {
        let text = item
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }
        let offsets = item.get("offsets");
        let start_secs = offsets
            .and_then(|value| value.get("from"))
            .and_then(serde_json::Value::as_f64)
            .map(|millis| millis as f32 / 1000.0);
        let end_secs = offsets
            .and_then(|value| value.get("to"))
            .and_then(serde_json::Value::as_f64)
            .map(|millis| millis as f32 / 1000.0);
        segments.push(Segment {
            segment: segments.len() as u32,
            text,
            start_secs,
            end_secs,
        });
    }
    Ok(Transcript {
        text: join_segments(&segments),
        segments,
        duration_secs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_output_preserves_whisper_cpp_segments() {
        let body = r#"{
            "result": {"language": "pt"},
            "transcription": [
                {"offsets": {"from": 0, "to": 1250}, "text": " Olá mundo."},
                {"offsets": {"from": 1250, "to": 2500}, "text": " Tudo bem?"}
            ]
        }"#;
        let actual = parse_output(body, 2.5).expect("valid whisper.cpp output");
        let expected = Transcript {
            text: "Olá mundo. Tudo bem?".to_string(),
            segments: vec![
                Segment {
                    segment: 0,
                    text: "Olá mundo.".to_string(),
                    start_secs: Some(0.0),
                    end_secs: Some(1.25),
                },
                Segment {
                    segment: 1,
                    text: "Tudo bem?".to_string(),
                    start_secs: Some(1.25),
                    end_secs: Some(2.5),
                },
            ],
            duration_secs: 2.5,
        };
        assert_eq!(actual, expected);
    }

    #[test]
    fn parse_output_rejects_an_unrelated_json_document() {
        let error = parse_output(r#"{"result": {"language": "pt"}}"#, 1.0)
            .expect_err("missing transcription must fail");
        assert_eq!(error, "whisper.cpp output has no `transcription` array");
    }

    #[cfg(unix)]
    #[test]
    fn command_path_rejects_a_regular_non_executable_file() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temporary directory");
        let command = temp.path().join("whisper-cli");
        std::fs::write(&command, b"not executable").expect("write command candidate");
        let mut permissions = std::fs::metadata(&command)
            .expect("command candidate metadata")
            .permissions();
        permissions.set_mode(0o644);
        std::fs::set_permissions(&command, permissions).expect("set non-executable mode");

        assert_eq!(command_path(command.to_str().expect("UTF-8 path")), None);
    }
}
