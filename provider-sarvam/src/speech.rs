//! `provider::sarvam::transcribe` and `provider::sarvam::speak`: Saaras
//! speech to text and Bulbul text to speech on the same router-resolved
//! credential as chat. Sarvam's speech endpoints live at the API root
//! (`https://api.sarvam.ai/speech-to-text`), not under the `/v1` the chat
//! endpoint uses, so their URLs derive from the configured chat `api_url`'s
//! origin.

use base64::Engine as _;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::types::events::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{config_from_resolve, ConfigError, SarvamConfig};
use crate::errors::classify_bus_error;
use crate::state;

pub const DEFAULT_TRANSCRIBE_MODEL: &str = "saaras:v3";
pub const DEFAULT_SPEAK_MODEL: &str = "bulbul:v3";
pub const DEFAULT_LANGUAGE: &str = "en-IN";
const DEFAULT_SPEAKER_V3: &str = "shubh";
const DEFAULT_SPEAKER_V2: &str = "anushka";
const SPEECH_ORIGIN: &str = "https://api.sarvam.ai";
const TRANSCRIBE_TIMEOUT_SECS: u64 = 280;
const SPEAK_TIMEOUT_SECS: u64 = 110;
/// The REST endpoint takes recordings this long; longer ones need batch.
const MAX_AUDIO_BYTES: usize = 50 * 1024 * 1024;
/// Words further apart than this start a new transcript segment.
const SEGMENT_GAP_SECS: f64 = 0.8;

/// The speech endpoints share the chat endpoint's origin: a residency host
/// (`api.in.sarvam.ai`) or a gateway configured for chat serves speech at
/// the same place.
pub fn speech_origin(chat_api_url: &str) -> String {
    match reqwest::Url::parse(chat_api_url) {
        Ok(url) => match (url.scheme(), url.host_str()) {
            (scheme, Some(host)) if matches!(scheme, "http" | "https") => {
                let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
                format!("{scheme}://{host}{port}")
            }
            _ => SPEECH_ORIGIN.to_string(),
        },
        Err(_) => SPEECH_ORIGIN.to_string(),
    }
}

/// A caller's language hint as Sarvam names it: `hi` and `hi-IN` both
/// become `hi-IN`, `en-US` becomes `en-IN`, Odia's ISO code `or` becomes
/// `od-IN`. Anything unknown is `None`, which lets Saaras detect the
/// language and makes Bulbul fall back to English.
pub fn sarvam_language(hint: Option<&str>) -> Option<String> {
    let hint = hint.map(str::trim).filter(|h| !h.is_empty())?;
    let lower = hint.to_ascii_lowercase();
    if lower == "unknown" {
        return None;
    }
    let base = lower.split(['-', '_']).next().unwrap_or("");
    let code = match base {
        "or" => "od",
        "as" | "bn" | "brx" | "doi" | "en" | "gu" | "hi" | "kn" | "kok" | "ks" | "mai" | "ml"
        | "mni" | "mr" | "ne" | "od" | "pa" | "sa" | "sat" | "sd" | "ta" | "te" | "ur" => base,
        _ => return None,
    };
    Some(format!("{code}-IN"))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TranscribeRequest {
    /// Speech-to-text model id; `saaras:v3` when omitted.
    #[serde(default)]
    pub model: Option<String>,
    /// The audio, base64. WAV, MP3, AAC, OGG, OPUS, FLAC, M4A, WebM.
    pub audio_base64: String,
    /// MIME type of the audio; `audio/wav` when omitted.
    #[serde(default)]
    pub mime: Option<String>,
    /// Language hint (`hi`, `hi-IN`, ...); detected when omitted.
    #[serde(default)]
    pub language: Option<String>,
    /// Accepted for the shared contract; Saaras takes no prompt.
    #[serde(default)]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TranscriptSegment {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_secs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_secs: Option<f64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TranscribeResponse {
    pub model: String,
    pub text: String,
    /// Sentences, cut at punctuation or a pause, with word timings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<TranscriptSegment>,
    /// Language Saaras detected or was told, as `hi-IN`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpeakRequest {
    /// Text-to-speech model id; `bulbul:v3` when omitted.
    #[serde(default)]
    pub model: Option<String>,
    /// Text to speak, up to 2500 characters. Write Indic words in their own
    /// script for natural speech.
    pub text: String,
    /// Speaker name (`shubh`, `shreya`, `manan`, `ishita`, ...; the v2
    /// voices `anushka`, `abhilash`, ... only with `bulbul:v2`).
    #[serde(default)]
    pub voice: Option<String>,
    /// `mp3` (default), `wav`, `pcm16`, `opus`.
    #[serde(default)]
    pub format: Option<String>,
    /// Language of the text (`hi`, `hi-IN`, ...); English when omitted.
    #[serde(default)]
    pub language: Option<String>,
    /// Speaking-rate multiplier, 0.5 to 2.0; 1.0 is the voice's own pace.
    #[serde(default)]
    pub speed: Option<f64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SpeakResponse {
    pub model: String,
    pub audio_base64: String,
    pub mime: String,
    /// The speaker used.
    pub voice: String,
}

#[derive(Debug, Default, Deserialize)]
struct WireTimestamps {
    #[serde(default)]
    words: Vec<String>,
    #[serde(default)]
    start_time_seconds: Vec<f64>,
    #[serde(default)]
    end_time_seconds: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct WireTranscript {
    #[serde(default)]
    transcript: String,
    #[serde(default)]
    language_code: Option<String>,
    #[serde(default)]
    timestamps: Option<WireTimestamps>,
}

#[derive(Debug, Deserialize)]
struct WireSpeech {
    #[serde(default)]
    audios: Vec<String>,
}

fn extension_for(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or("").trim() {
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/webm" | "video/webm" => "webm",
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => "m4a",
        "audio/aac" => "aac",
        _ => "wav",
    }
}

/// Bulbul's `output_audio_codec`, the sample rate to ask for, and the MIME
/// the caller gets.
fn speak_format(format: Option<&str>) -> Result<(&'static str, u32, &'static str), Error> {
    match format
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .unwrap_or("mp3")
    {
        "mp3" => Ok(("mp3", 24_000, "audio/mpeg")),
        "wav" => Ok(("wav", 24_000, "audio/wav")),
        "pcm16" | "pcm" => Ok(("linear16", 16_000, "audio/pcm")),
        "opus" => Ok(("opus", 24_000, "audio/ogg")),
        other => Err(Error::Handler(format!(
            "provider/invalid_input: format `{other}` is not one of mp3, wav, pcm16, opus"
        ))),
    }
}

/// Sentences out of Saaras' word timings: cut at sentence punctuation or a
/// pause longer than [`SEGMENT_GAP_SECS`].
fn segments_from_timestamps(t: &WireTimestamps) -> Vec<TranscriptSegment> {
    let mut out = Vec::new();
    let mut words: Vec<&str> = Vec::new();
    let mut start: Option<f64> = None;
    let mut end: Option<f64> = None;
    let flush = |words: &mut Vec<&str>,
                 start: &mut Option<f64>,
                 end: &mut Option<f64>,
                 out: &mut Vec<TranscriptSegment>| {
        if !words.is_empty() {
            out.push(TranscriptSegment {
                text: words.join(" "),
                start_secs: *start,
                end_secs: *end,
            });
        }
        words.clear();
        *start = None;
        *end = None;
    };
    for (i, word) in t.words.iter().enumerate() {
        let word = word.trim();
        if word.is_empty() {
            continue;
        }
        let this_start = t.start_time_seconds.get(i).copied();
        let this_end = t.end_time_seconds.get(i).copied();
        if let (Some(prev_end), Some(this_start)) = (end, this_start) {
            if this_start - prev_end > SEGMENT_GAP_SECS {
                flush(&mut words, &mut start, &mut end, &mut out);
            }
        }
        if start.is_none() {
            start = this_start;
        }
        words.push(word);
        end = this_end.or(end);
        if word.ends_with(['.', '?', '!', '।', '॥']) {
            flush(&mut words, &mut start, &mut end, &mut out);
        }
    }
    flush(&mut words, &mut start, &mut end, &mut out);
    out
}

async fn resolve(
    iii: &IIIClient,
    cache: &ScaffoldCache,
    model: &str,
) -> Result<SarvamConfig, Error> {
    let token = cache.load_token(iii, state::STATE_SCOPE).await;
    let resolved = match cache
        .resolve(
            iii,
            crate::PROVIDER_ID,
            token.as_deref(),
            Some(crate::register::CREDENTIAL_ENV_VAR),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            if classify_bus_error(&e) == ErrorKind::AuthExpired {
                cache.invalidate();
            }
            return Err(e);
        }
    };
    config_from_resolve(model, None, &resolved).map_err(|e| match e {
        ConfigError::NotConfigured => Error::Handler(
            "provider/not_configured: no api_key in the llm-router entry for sarvam \
             (set SARVAM_API_KEY or paste the key in Settings)"
                .into(),
        ),
        other => Error::Handler(format!("provider/config: {other}")),
    })
}

async fn failure(response: reqwest::Response) -> Error {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    crate::errors::upstream_status(status, &body)
}

pub async fn transcribe(
    iii: &IIIClient,
    http: &reqwest::Client,
    cache: &ScaffoldCache,
    req: TranscribeRequest,
) -> Result<TranscribeResponse, Error> {
    let encoded = req.audio_base64.trim();
    if encoded.len() > MAX_AUDIO_BYTES / 3 * 4 + 4 {
        return Err(Error::Handler(format!(
            "provider/invalid_input: audio_base64 is {} characters, more than a {MAX_AUDIO_BYTES}-byte file encodes to",
            encoded.len()
        )));
    }
    let audio = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| {
            Error::Handler(format!(
                "provider/invalid_input: audio_base64 is not base64: {e}"
            ))
        })?;
    if audio.is_empty() {
        return Err(Error::Handler(
            "provider/invalid_input: audio is empty".into(),
        ));
    }
    if audio.len() > MAX_AUDIO_BYTES {
        return Err(Error::Handler(format!(
            "provider/invalid_input: audio is {} bytes, over the {MAX_AUDIO_BYTES}-byte limit",
            audio.len()
        )));
    }
    let model = req
        .model
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_TRANSCRIBE_MODEL.to_string());
    let cfg = resolve(iii, cache, &model).await?;
    let mime = req.mime.unwrap_or_else(|| "audio/wav".to_string());
    let file = reqwest::multipart::Part::bytes(audio)
        .file_name(format!("audio.{}", extension_for(&mime)))
        .mime_str(mime.split(';').next().unwrap_or("audio/wav").trim())
        .map_err(|e| Error::Handler(format!("provider/invalid_input: mime `{mime}`: {e}")))?;
    let mut form = reqwest::multipart::Form::new()
        .part("file", file)
        .text("model", model.clone())
        .text("with_timestamps", "true");
    if model.starts_with("saaras") {
        form = form.text("mode", "transcribe");
    }
    if let Some(language) = sarvam_language(req.language.as_deref()) {
        form = form.text("language_code", language);
    }
    let response = http
        .post(format!("{}/speech-to-text", speech_origin(&cfg.api_url)))
        .header("api-subscription-key", &cfg.credential_value)
        .timeout(std::time::Duration::from_secs(TRANSCRIBE_TIMEOUT_SECS))
        .multipart(form)
        .send()
        .await
        .map_err(|e| Error::Handler(format!("provider/upstream: {e}")))?;
    if !response.status().is_success() {
        return Err(failure(response).await);
    }
    let wire: WireTranscript = response
        .json()
        .await
        .map_err(|e| Error::Handler(format!("provider/bad_response: {e}")))?;
    let timestamps = wire.timestamps.unwrap_or_default();
    let duration_secs = timestamps
        .end_time_seconds
        .iter()
        .copied()
        .fold(None, |acc: Option<f64>, e| {
            Some(acc.map_or(e, |a| a.max(e)))
        });
    Ok(TranscribeResponse {
        model,
        text: wire.transcript.trim().to_string(),
        segments: segments_from_timestamps(&timestamps),
        language: wire.language_code,
        duration_secs,
    })
}

pub async fn speak(
    iii: &IIIClient,
    http: &reqwest::Client,
    cache: &ScaffoldCache,
    req: SpeakRequest,
) -> Result<SpeakResponse, Error> {
    let text = req.text.trim();
    if text.is_empty() {
        return Err(Error::Handler(
            "provider/invalid_input: text is empty".into(),
        ));
    }
    let model = req
        .model
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_SPEAK_MODEL.to_string());
    let voice = req
        .voice
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if model.starts_with("bulbul:v2") {
                DEFAULT_SPEAKER_V2.to_string()
            } else {
                DEFAULT_SPEAKER_V3.to_string()
            }
        });
    let (codec, sample_rate, mime) = speak_format(req.format.as_deref())?;
    let language =
        sarvam_language(req.language.as_deref()).unwrap_or_else(|| DEFAULT_LANGUAGE.to_string());
    let cfg = resolve(iii, cache, &model).await?;
    let mut body = serde_json::json!({
        "text": text,
        "target_language_code": language,
        "speaker": voice,
        "model": model,
        "speech_sample_rate": sample_rate,
        "output_audio_codec": codec,
    });
    if let Some(speed) = req.speed {
        body["pace"] = serde_json::json!(speed);
    }
    let response = http
        .post(format!("{}/text-to-speech", speech_origin(&cfg.api_url)))
        .header("api-subscription-key", &cfg.credential_value)
        .timeout(std::time::Duration::from_secs(SPEAK_TIMEOUT_SECS))
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Handler(format!("provider/upstream: {e}")))?;
    if !response.status().is_success() {
        return Err(failure(response).await);
    }
    let wire: WireSpeech = response
        .json()
        .await
        .map_err(|e| Error::Handler(format!("provider/bad_response: {e}")))?;
    let audio_base64 = wire
        .audios
        .into_iter()
        .find(|a| !a.is_empty())
        .ok_or_else(|| Error::Handler("provider/bad_response: no audio in the reply".into()))?;
    Ok(SpeakResponse {
        model,
        audio_base64,
        mime: mime.to_string(),
        voice,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_urls_share_the_chat_origin() {
        assert_eq!(
            speech_origin("https://api.sarvam.ai/v1/chat/completions"),
            "https://api.sarvam.ai"
        );
        assert_eq!(
            speech_origin("https://api.in.sarvam.ai/v1/chat/completions"),
            "https://api.in.sarvam.ai"
        );
        assert_eq!(
            speech_origin("http://127.0.0.1:8080/v1/chat/completions"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(speech_origin("not a url"), SPEECH_ORIGIN);
    }

    #[test]
    fn language_hints_become_sarvam_codes() {
        assert_eq!(sarvam_language(Some("hi")).as_deref(), Some("hi-IN"));
        assert_eq!(sarvam_language(Some("hi-IN")).as_deref(), Some("hi-IN"));
        assert_eq!(sarvam_language(Some("en-US")).as_deref(), Some("en-IN"));
        assert_eq!(sarvam_language(Some("or")).as_deref(), Some("od-IN"));
        assert_eq!(sarvam_language(Some("sat")).as_deref(), Some("sat-IN"));
        assert_eq!(sarvam_language(Some("fr")), None);
        assert_eq!(sarvam_language(Some("unknown")), None);
        assert_eq!(sarvam_language(None), None);
    }

    #[test]
    fn formats_map_to_bulbul_codecs() {
        assert_eq!(speak_format(None).unwrap(), ("mp3", 24_000, "audio/mpeg"));
        assert_eq!(
            speak_format(Some("wav")).unwrap(),
            ("wav", 24_000, "audio/wav")
        );
        assert_eq!(
            speak_format(Some("pcm16")).unwrap(),
            ("linear16", 16_000, "audio/pcm")
        );
        assert_eq!(
            speak_format(Some("opus")).unwrap(),
            ("opus", 24_000, "audio/ogg")
        );
        assert!(speak_format(Some("aiff")).is_err());
    }

    #[test]
    fn timestamps_become_sentences() {
        let t = WireTimestamps {
            words: vec![
                "नमस्ते".into(),
                "दुनिया।".into(),
                "How".into(),
                "are".into(),
                "you?".into(),
                "Late".into(),
            ],
            start_time_seconds: vec![0.0, 0.5, 1.2, 1.5, 1.8, 4.0],
            end_time_seconds: vec![0.4, 0.9, 1.4, 1.7, 2.1, 4.4],
        };
        let segments = segments_from_timestamps(&t);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].text, "नमस्ते दुनिया।");
        assert_eq!(segments[0].end_secs, Some(0.9));
        assert_eq!(segments[1].text, "How are you?");
        assert_eq!(segments[2].text, "Late");
        assert_eq!(segments[2].start_secs, Some(4.0));
    }

    #[test]
    fn transcript_reply_parses_with_and_without_timestamps() {
        let raw = r#"{"request_id":"r1","transcript":"नमस्ते दुनिया","language_code":"hi-IN","timestamps":{"words":["नमस्ते","दुनिया"],"start_time_seconds":[0.0,0.5],"end_time_seconds":[0.4,0.9]}}"#;
        let wire: WireTranscript = serde_json::from_str(raw).unwrap();
        assert_eq!(wire.transcript, "नमस्ते दुनिया");
        assert_eq!(wire.timestamps.unwrap().words.len(), 2);
        let bare: WireTranscript = serde_json::from_str(r#"{"transcript":"hi"}"#).unwrap();
        assert!(bare.timestamps.is_none());
        let audio: WireSpeech =
            serde_json::from_str(r#"{"request_id":"r2","audios":["AAAA"]}"#).unwrap();
        assert_eq!(audio.audios[0], "AAAA");
    }
}
