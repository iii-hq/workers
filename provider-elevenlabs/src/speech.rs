//! `provider::elevenlabs::transcribe` and `provider::elevenlabs::speak`:
//! the ElevenLabs speech-to-text (Scribe) and text-to-speech endpoints on
//! the router-resolved credential. Voices are addressed by id or by name;
//! names resolve through `GET /v1/voices` once per process and again on a
//! miss.

use std::sync::Arc;

use base64::Engine as _;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::types::credential::Credential;
use llm_router::types::events::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::errors::{classify_bus_error, upstream_status};
use crate::state;

pub const DEFAULT_API_URL: &str = "https://api.elevenlabs.io/v1";
pub const DEFAULT_TRANSCRIBE_MODEL: &str = "scribe_v1";
pub const DEFAULT_SPEAK_MODEL: &str = "eleven_multilingual_v2";
/// George, one of the premade voices every account has.
pub const DEFAULT_VOICE_ID: &str = "JBFqnCBsd6RMkjVDRZzb";
/// Scribe accepts files up to 5 GB; the bus does not, and a transcript of
/// this much audio takes minutes.
const MAX_AUDIO_BYTES: usize = 100 * 1024 * 1024;
const TRANSCRIBE_TIMEOUT_SECS: u64 = 280;
const SPEAK_TIMEOUT_SECS: u64 = 110;
/// Words further apart than this start a new transcript segment.
const SEGMENT_GAP_SECS: f64 = 0.8;

/// The secret inside a resolved credential: the API key, or the OAuth
/// access token for accounts that log in instead.
pub fn secret_of(credential: &Credential) -> &str {
    match credential {
        Credential::ApiKey { key } => key,
        Credential::Oauth { access_token, .. } => access_token,
    }
}

pub fn api_url_or_default(api_url: Option<&str>) -> String {
    api_url
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .unwrap_or(DEFAULT_API_URL)
        .trim_end_matches('/')
        .to_string()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TranscribeRequest {
    /// Speech-to-text model id; `scribe_v1` when omitted.
    #[serde(default)]
    pub model: Option<String>,
    /// The audio, base64. Any common container; `mime` names it.
    pub audio_base64: String,
    /// MIME type of the audio; `audio/wav` when omitted.
    #[serde(default)]
    pub mime: Option<String>,
    /// ISO 639-1 or 639-3 language code; detected when omitted.
    #[serde(default)]
    pub language: Option<String>,
    /// Accepted for the shared contract; Scribe takes no prompt.
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
    /// Language Scribe detected (ISO 639-3, e.g. `eng`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpeakRequest {
    /// Text-to-speech model id; `eleven_multilingual_v2` when omitted.
    #[serde(default)]
    pub model: Option<String>,
    /// Text to speak.
    pub text: String,
    /// Voice id, or the name of a voice on the account; George when omitted.
    #[serde(default)]
    pub voice: Option<String>,
    /// `mp3` (default), `wav`, `pcm16`, `opus`.
    #[serde(default)]
    pub format: Option<String>,
    /// ISO 639-1 language to enforce; ignored by models that pick it from
    /// the text.
    #[serde(default)]
    pub language: Option<String>,
    /// Speaking-rate multiplier, 0.7 to 1.2; 1.0 is the voice's own pace.
    #[serde(default)]
    pub speed: Option<f64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SpeakResponse {
    pub model: String,
    pub audio_base64: String,
    pub mime: String,
    /// The voice id used.
    pub voice: String,
}

#[derive(Debug, Deserialize)]
struct WireWord {
    #[serde(default)]
    text: String,
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireTranscript {
    #[serde(default)]
    language_code: Option<String>,
    #[serde(default)]
    text: String,
    #[serde(default)]
    words: Vec<WireWord>,
}

#[derive(Debug, Deserialize)]
struct WireVoice {
    voice_id: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireVoices {
    #[serde(default)]
    voices: Vec<WireVoice>,
}

/// Per-process voice directory: name → id, fetched lazily.
#[derive(Default)]
pub struct Voices {
    known: Mutex<Vec<(String, String)>>,
}

impl Voices {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    async fn refresh(
        &self,
        http: &reqwest::Client,
        api_url: &str,
        credential: &str,
    ) -> Result<(), Error> {
        let response = http
            .get(format!("{api_url}/voices"))
            .header("xi-api-key", credential)
            .send()
            .await
            .map_err(|e| Error::Handler(format!("provider/upstream: voices fetch failed: {e}")))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let body = response.text().await.unwrap_or_default();
            return Err(upstream_status(status, &body));
        }
        let wire: WireVoices = response
            .json()
            .await
            .map_err(|e| Error::Handler(format!("provider/bad_response: voices list: {e}")))?;
        let mut known = self.known.lock().await;
        *known = wire
            .voices
            .into_iter()
            .map(|v| (v.name.unwrap_or_default(), v.voice_id))
            .collect();
        Ok(())
    }

    async fn lookup(&self, name: &str) -> Option<String> {
        let wanted = name.trim().to_ascii_lowercase();
        self.known
            .lock()
            .await
            .iter()
            .find(|(n, _)| {
                let n = n.to_ascii_lowercase();
                n == wanted || n.split(" - ").next().map(str::trim) == Some(wanted.as_str())
            })
            .map(|(_, id)| id.clone())
    }

    /// A voice id for `voice`: ids pass through, names resolve against the
    /// account's voices (refreshing the directory once on a miss).
    pub async fn resolve(
        &self,
        http: &reqwest::Client,
        api_url: &str,
        credential: &str,
        voice: &str,
    ) -> Result<String, Error> {
        if looks_like_voice_id(voice) {
            return Ok(voice.to_string());
        }
        if let Some(id) = self.lookup(voice).await {
            return Ok(id);
        }
        self.refresh(http, api_url, credential).await?;
        self.lookup(voice).await.ok_or_else(|| {
            Error::Handler(format!(
                "provider/invalid_input: no voice named \"{voice}\" on this ElevenLabs account; \
                 pass a voice id or a name from GET /v1/voices"
            ))
        })
    }
}

/// ElevenLabs voice ids are 20 alphanumeric characters.
fn looks_like_voice_id(value: &str) -> bool {
    value.len() == 20 && value.chars().all(|c| c.is_ascii_alphanumeric())
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

/// The ElevenLabs `output_format`, the MIME the caller gets, and whether
/// the raw PCM must be wrapped into a WAV container first.
fn speak_format(format: Option<&str>) -> Result<(&'static str, &'static str, bool), Error> {
    match format
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .unwrap_or("mp3")
    {
        "mp3" => Ok(("mp3_44100_128", "audio/mpeg", false)),
        "wav" => Ok(("pcm_16000", "audio/wav", true)),
        "pcm16" | "pcm" => Ok(("pcm_16000", "audio/pcm", false)),
        "opus" => Ok(("opus_48000_64", "audio/ogg", false)),
        other => Err(Error::Handler(format!(
            "provider/invalid_input: format `{other}` is not one of mp3, wav, pcm16, opus"
        ))),
    }
}

/// A 16 kHz mono 16-bit WAV around raw little-endian PCM.
fn wav_from_pcm16(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let byte_rate = sample_rate * 2;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

/// Sentences out of Scribe's word list: cut at sentence punctuation or a
/// pause longer than [`SEGMENT_GAP_SECS`]. Spacing entries join words;
/// audio events (laughter, music) are dropped from the text.
fn segments_from_words(words: &[WireWord]) -> Vec<TranscriptSegment> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut start: Option<f64> = None;
    let mut end: Option<f64> = None;
    let flush = |text: &mut String,
                 start: &mut Option<f64>,
                 end: &mut Option<f64>,
                 out: &mut Vec<TranscriptSegment>| {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            out.push(TranscriptSegment {
                text: trimmed.to_string(),
                start_secs: *start,
                end_secs: *end,
            });
        }
        text.clear();
        *start = None;
        *end = None;
    };
    for word in words {
        match word.kind.as_deref() {
            Some("audio_event") => continue,
            Some("spacing") => {
                text.push_str(&word.text);
                continue;
            }
            _ => {}
        }
        if let (Some(prev_end), Some(this_start)) = (end, word.start) {
            if this_start - prev_end > SEGMENT_GAP_SECS {
                flush(&mut text, &mut start, &mut end, &mut out);
            }
        }
        if start.is_none() {
            start = word.start;
        }
        text.push_str(&word.text);
        end = word.end.or(end);
        if word
            .text
            .trim_end()
            .ends_with(['.', '?', '!', '。', '？', '！'])
        {
            flush(&mut text, &mut start, &mut end, &mut out);
        }
    }
    flush(&mut text, &mut start, &mut end, &mut out);
    out
}

struct Resolved {
    credential: String,
    api_url: String,
}

async fn resolve(iii: &IIIClient, cache: &ScaffoldCache) -> Result<Resolved, Error> {
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
    let credential = resolved
        .credential
        .as_ref()
        .map(secret_of)
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            Error::Handler(
                "provider/not_configured: no api_key in the llm-router entry for elevenlabs \
                 (set ELEVENLABS_API_KEY or paste the key in Settings)"
                    .into(),
            )
        })?;
    Ok(Resolved {
        credential,
        api_url: api_url_or_default(resolved.api_url.as_deref()),
    })
}

pub async fn transcribe(
    iii: &IIIClient,
    http: &reqwest::Client,
    cache: &ScaffoldCache,
    req: TranscribeRequest,
) -> Result<TranscribeResponse, Error> {
    let audio = base64::engine::general_purpose::STANDARD
        .decode(req.audio_base64.trim())
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
    let Resolved {
        credential,
        api_url,
    } = resolve(iii, cache).await?;
    let mime = req.mime.unwrap_or_else(|| "audio/wav".to_string());
    let file = reqwest::multipart::Part::bytes(audio)
        .file_name(format!("audio.{}", extension_for(&mime)))
        .mime_str(mime.split(';').next().unwrap_or("audio/wav").trim())
        .map_err(|e| Error::Handler(format!("provider/invalid_input: mime `{mime}`: {e}")))?;
    let mut form = reqwest::multipart::Form::new()
        .part("file", file)
        .text("model_id", model.clone())
        .text("timestamps_granularity", "word")
        .text("diarize", "false")
        .text("tag_audio_events", "false");
    if let Some(language) = req.language.filter(|l| !l.trim().is_empty()) {
        form = form.text("language_code", language);
    }
    let response = http
        .post(format!("{api_url}/speech-to-text"))
        .header("xi-api-key", &credential)
        .timeout(std::time::Duration::from_secs(TRANSCRIBE_TIMEOUT_SECS))
        .multipart(form)
        .send()
        .await
        .map_err(|e| Error::Handler(format!("provider/upstream: {e}")))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let body = response.text().await.unwrap_or_default();
        return Err(upstream_status(status, &body));
    }
    let wire: WireTranscript = response
        .json()
        .await
        .map_err(|e| Error::Handler(format!("provider/bad_response: {e}")))?;
    let duration_secs = wire
        .words
        .iter()
        .filter_map(|w| w.end)
        .fold(None, |acc: Option<f64>, e| {
            Some(acc.map_or(e, |a| a.max(e)))
        });
    Ok(TranscribeResponse {
        model,
        text: wire.text.trim().to_string(),
        segments: segments_from_words(&wire.words),
        language: wire.language_code,
        duration_secs,
    })
}

pub async fn speak(
    iii: &IIIClient,
    http: &reqwest::Client,
    cache: &ScaffoldCache,
    voices: &Voices,
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
    let (output_format, mime, wrap_wav) = speak_format(req.format.as_deref())?;
    let Resolved {
        credential,
        api_url,
    } = resolve(iii, cache).await?;
    let voice_id = match req
        .voice
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(voice) => voices.resolve(http, &api_url, &credential, voice).await?,
        None => DEFAULT_VOICE_ID.to_string(),
    };
    let mut body = serde_json::json!({ "text": text, "model_id": model });
    if let Some(language) = req.language.filter(|l| !l.trim().is_empty()) {
        if model != "eleven_multilingual_v2" {
            body["language_code"] = serde_json::json!(language);
        }
    }
    if let Some(speed) = req.speed {
        body["voice_settings"] = serde_json::json!({ "speed": speed });
    }
    let response = http
        .post(format!(
            "{api_url}/text-to-speech/{voice_id}?output_format={output_format}"
        ))
        .header("xi-api-key", &credential)
        .timeout(std::time::Duration::from_secs(SPEAK_TIMEOUT_SECS))
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Handler(format!("provider/upstream: {e}")))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let body = response.text().await.unwrap_or_default();
        return Err(upstream_status(status, &body));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| Error::Handler(format!("provider/bad_response: {e}")))?;
    if bytes.is_empty() {
        return Err(Error::Handler(
            "provider/bad_response: empty audio body".into(),
        ));
    }
    let audio = if wrap_wav {
        wav_from_pcm16(&bytes, 16_000)
    } else {
        bytes.to_vec()
    };
    Ok(SpeakResponse {
        model,
        audio_base64: base64::engine::general_purpose::STANDARD.encode(audio),
        mime: mime.to_string(),
        voice: voice_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, start: f64, end: f64) -> WireWord {
        WireWord {
            text: text.into(),
            start: Some(start),
            end: Some(end),
            kind: Some("word".into()),
        }
    }

    fn space() -> WireWord {
        WireWord {
            text: " ".into(),
            start: None,
            end: None,
            kind: Some("spacing".into()),
        }
    }

    #[test]
    fn words_become_sentences_at_punctuation_and_pauses() {
        let words = vec![
            word("Hello", 0.0, 0.4),
            space(),
            word("there.", 0.5, 0.9),
            space(),
            word("Second", 1.0, 1.3),
            space(),
            word("bit", 1.4, 1.6),
            space(),
            word("later", 3.0, 3.4),
        ];
        let segments = segments_from_words(&words);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].text, "Hello there.");
        assert_eq!(segments[0].start_secs, Some(0.0));
        assert_eq!(segments[0].end_secs, Some(0.9));
        assert_eq!(segments[1].text, "Second bit");
        assert_eq!(segments[2].text, "later");
        assert_eq!(segments[2].start_secs, Some(3.0));
    }

    #[test]
    fn audio_events_are_dropped_from_segments() {
        let words = vec![
            WireWord {
                text: "(laughter)".into(),
                start: Some(0.0),
                end: Some(0.5),
                kind: Some("audio_event".into()),
            },
            word("Hi", 0.6, 0.8),
        ];
        let segments = segments_from_words(&words);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "Hi");
    }

    #[test]
    fn formats_and_voice_ids() {
        assert_eq!(
            speak_format(None).unwrap(),
            ("mp3_44100_128", "audio/mpeg", false)
        );
        assert_eq!(
            speak_format(Some("wav")).unwrap(),
            ("pcm_16000", "audio/wav", true)
        );
        assert_eq!(
            speak_format(Some("opus")).unwrap(),
            ("opus_48000_64", "audio/ogg", false)
        );
        assert!(speak_format(Some("aiff")).is_err());
        assert!(looks_like_voice_id("JBFqnCBsd6RMkjVDRZzb"));
        assert!(!looks_like_voice_id("George"));
        assert_eq!(extension_for("audio/webm;codecs=opus"), "webm");
        assert_eq!(api_url_or_default(None), DEFAULT_API_URL);
        assert_eq!(
            api_url_or_default(Some(" https://api.eu.residency.elevenlabs.io/v1/ ")),
            "https://api.eu.residency.elevenlabs.io/v1"
        );
    }

    #[test]
    fn wav_header_describes_the_pcm() {
        let wav = wav_from_pcm16(&[0u8; 32], 16_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 32);
        assert_eq!(wav.len(), 76);
    }

    #[test]
    fn scribe_reply_parses() {
        let raw = r#"{"language_code":"eng","language_probability":0.98,"text":"Hello there.","words":[{"text":"Hello","start":0.0,"end":0.4,"type":"word"},{"text":" ","type":"spacing"},{"text":"there.","start":0.5,"end":0.9,"type":"word"}]}"#;
        let wire: WireTranscript = serde_json::from_str(raw).unwrap();
        assert_eq!(wire.text, "Hello there.");
        assert_eq!(wire.words.len(), 3);
        assert_eq!(wire.language_code.as_deref(), Some("eng"));
    }
}
