//! The speech-to-text engines behind every function: the local recognizers
//! (loaded once per model and endpointing configuration, shared by every
//! session) and the OpenAI-compatible transcription endpoint.
//!
//! Local recognition is two passes. A small streaming transducer decodes as
//! audio arrives and produces the live partial text plus the utterance
//! boundaries; it runs in ~1/60th of real time, so a 100 ms dictation chunk
//! costs a couple of milliseconds and is decoded inline. A large offline
//! transducer then re-decodes each finished utterance for the final text,
//! with punctuation and casing, in about a tenth of real time. The second
//! pass is optional: while its model is still downloading, the streaming
//! text stands.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use sherpa_onnx::{
    OfflineModelConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig,
    OnlineModelConfig, OnlineRecognizer, OnlineRecognizerConfig, OnlineStream,
    OnlineTransducerModelConfig,
};
use tokio::sync::Mutex;

use crate::audio::{self, TARGET_SAMPLE_RATE};
use crate::config::{SttBackend, WorkerConfig};
use crate::models::{self, ModelKind, ModelSpec, ProgressSink};

/// Samples fed ahead of the first real audio of every stream. A streaming
/// zipformer needs a second or so of context before its first words come out
/// right; digital silence does not count, so this is faint noise.
pub const PREROLL_SAMPLES: usize = 24_000;
const PREROLL_AMPLITUDE: f32 = 0.002;
/// Samples fed per decode step for whole-buffer transcription.
const FILE_CHUNK: usize = 1_600;
/// Audio kept around an utterance for the second pass, in samples: a little
/// before the first token and after the last so clipped consonants survive.
const REFINE_PAD_SAMPLES: usize = 4_000;

/// One committed stretch of speech.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Segment {
    pub segment: u32,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_secs: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_secs: Option<f32>,
}

/// A whole-buffer transcription.
#[derive(Debug, Clone, PartialEq, serde::Serialize, schemars::JsonSchema)]
pub struct Transcript {
    pub text: String,
    pub segments: Vec<Segment>,
    pub duration_secs: f32,
}

/// The streaming recognizer settings that require a reload when they change.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadKey {
    pub model: String,
    pub num_threads: usize,
    pub rule1: f32,
    pub rule2: f32,
    pub rule3: f32,
}

impl LoadKey {
    pub fn from_config(cfg: &WorkerConfig) -> Self {
        Self {
            model: cfg.stt.model.clone(),
            num_threads: cfg.stt.num_threads,
            rule1: cfg.stt.silence_without_speech_secs as f32,
            rule2: cfg.stt.silence_after_speech_secs as f32,
            rule3: cfg.stt.max_utterance_secs as f32,
        }
    }
}

/// The second-pass recognizer settings that require a reload when they change.
#[derive(Debug, Clone, PartialEq)]
pub struct FinalKey {
    pub model: String,
    pub num_threads: usize,
}

impl FinalKey {
    pub fn from_config(cfg: &WorkerConfig) -> Self {
        Self {
            model: cfg.stt.final_model.clone(),
            num_threads: cfg.stt.num_threads.max(4),
        }
    }
}

/// A loaded streaming recognizer plus what it was loaded with.
pub struct Loaded {
    recognizer: OnlineRecognizer,
    pub key: LoadKey,
    pub model_dir: PathBuf,
    pub load_ms: u128,
}

impl Loaded {
    /// A fresh stream, already primed with the preroll.
    pub fn open_stream(self: &Arc<Self>) -> Stream {
        let inner = self.recognizer.create_stream();
        inner.accept_waveform(TARGET_SAMPLE_RATE as i32, &preroll());
        Stream {
            loaded: self.clone(),
            inner,
            segment: 0,
            last_partial: String::new(),
            fed: PREROLL_SAMPLES,
        }
    }

    /// Transcribe a whole 16 kHz mono buffer with the streaming model only.
    pub fn transcribe(self: &Arc<Self>, samples: &[f32]) -> Transcript {
        let mut stream = self.open_stream();
        let mut segments = Vec::new();
        for chunk in samples.chunks(FILE_CHUNK) {
            segments.extend(stream.feed(chunk).finals);
        }
        segments.extend(stream.finish());
        Transcript {
            text: join_segments(&segments),
            segments,
            duration_secs: samples.len() as f32 / TARGET_SAMPLE_RATE as f32,
        }
    }
}

/// A loaded second-pass recognizer.
pub struct FinalLoaded {
    recognizer: OfflineRecognizer,
    pub key: FinalKey,
    pub load_ms: u128,
}

impl FinalLoaded {
    /// Decode one utterance. Empty when the model heard nothing.
    pub fn refine(&self, samples: &[f32]) -> String {
        if samples.is_empty() {
            return String::new();
        }
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(TARGET_SAMPLE_RATE as i32, samples);
        self.recognizer.decode(&stream);
        stream
            .get_result()
            .map(|r| r.text.trim().to_string())
            .unwrap_or_default()
    }

    /// Re-decode every segment of a streaming transcript, keeping its
    /// boundaries and timestamps. Each segment's window runs from the
    /// previous segment's end to the next segment's start (the whole buffer
    /// for a lone segment), so audio the streaming model missed at an
    /// utterance's onset still reaches the second pass.
    pub fn refine_transcript(&self, samples: &[f32], transcript: Transcript) -> Transcript {
        let to_index = |secs: f32| ((secs * TARGET_SAMPLE_RATE as f32) as usize).min(samples.len());
        let count = transcript.segments.len();
        let bounds: Vec<(usize, usize)> = (0..count)
            .map(|i| {
                let seg = &transcript.segments[i];
                let start = if i == 0 {
                    0
                } else {
                    transcript.segments[i - 1]
                        .end_secs
                        .map(to_index)
                        .map(|end| end.saturating_sub(REFINE_PAD_SAMPLES))
                        .or_else(|| seg.start_secs.map(to_index))
                        .unwrap_or(0)
                };
                let end = if i + 1 == count {
                    samples.len()
                } else {
                    transcript.segments[i + 1]
                        .start_secs
                        .map(to_index)
                        .map(|next| (next + REFINE_PAD_SAMPLES).min(samples.len()))
                        .or_else(|| seg.end_secs.map(to_index))
                        .unwrap_or(samples.len())
                };
                (start, end)
            })
            .collect();
        let segments: Vec<Segment> = transcript
            .segments
            .into_iter()
            .zip(bounds)
            .map(|(seg, (start, end))| {
                let refined = if start < end {
                    self.refine(&samples[start..end])
                } else {
                    String::new()
                };
                Segment {
                    text: if refined.is_empty() {
                        seg.text
                    } else {
                        refined
                    },
                    ..seg
                }
            })
            .collect();
        Transcript {
            text: join_segments(&segments),
            segments,
            duration_secs: transcript.duration_secs,
        }
    }
}

/// What one feed step produced.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Step {
    /// The in-progress text after this step, when it changed.
    pub partial: Option<String>,
    /// Segments committed by an endpoint during this step.
    pub finals: Vec<Segment>,
}

/// One decoding stream over a loaded streaming recognizer.
pub struct Stream {
    loaded: Arc<Loaded>,
    inner: OnlineStream,
    segment: u32,
    last_partial: String,
    fed: usize,
}

impl Stream {
    /// Seconds of audio fed so far, preroll excluded.
    pub fn fed_secs(&self) -> f32 {
        self.fed.saturating_sub(PREROLL_SAMPLES) as f32 / TARGET_SAMPLE_RATE as f32
    }

    pub fn segment(&self) -> u32 {
        self.segment
    }

    /// Feed 16 kHz mono samples and decode everything that became ready.
    pub fn feed(&mut self, samples: &[f32]) -> Step {
        self.inner
            .accept_waveform(TARGET_SAMPLE_RATE as i32, samples);
        self.fed += samples.len();
        self.decode_ready()
    }

    /// Flush the stream: decode the tail and commit whatever is pending.
    pub fn finish(&mut self) -> Vec<Segment> {
        self.inner.input_finished();
        let mut step = self.decode_ready();
        let (text, start, end) = self.current();
        if !text.trim().is_empty() {
            step.finals.push(self.commit(text, start, end));
        }
        step.finals
    }

    /// Decode everything that is ready and commit on an endpoint. An endpoint
    /// on an empty result is the recognizer hearing silence before any
    /// speech; resetting there would drop the model's lookahead and with it
    /// the first words of the utterance that follows, so only a non-empty
    /// result commits.
    fn decode_ready(&mut self) -> Step {
        let loaded = self.loaded.clone();
        let recognizer = &loaded.recognizer;
        while recognizer.is_ready(&self.inner) {
            recognizer.decode(&self.inner);
        }
        let mut step = Step::default();
        let (text, start, end) = self.current();
        if recognizer.is_endpoint(&self.inner) && !text.trim().is_empty() {
            let seg = self.commit(text, start, end);
            recognizer.reset(&self.inner);
            step.finals.push(seg);
            step.partial = Some(String::new());
            self.last_partial.clear();
            return step;
        }
        if text != self.last_partial {
            self.last_partial = text.clone();
            step.partial = Some(text);
        }
        step
    }

    /// The current in-progress text and its token time bounds, in seconds
    /// relative to the first real sample.
    fn current(&self) -> (String, Option<f32>, Option<f32>) {
        let Some(result) = self.loaded.recognizer.get_result(&self.inner) else {
            return (String::new(), None, None);
        };
        let text = normalize(&result.text);
        let bounds = result.timestamps.as_ref().and_then(|ts| {
            let first = *ts.first()?;
            let last = *ts.last()?;
            let offset = PREROLL_SAMPLES as f32 / TARGET_SAMPLE_RATE as f32;
            Some(((first - offset).max(0.0), (last - offset + 0.2).max(0.0)))
        });
        match bounds {
            Some((s, e)) => (text, Some(s), Some(e)),
            None => (text, None, None),
        }
    }

    fn commit(&mut self, text: String, start: Option<f32>, end: Option<f32>) -> Segment {
        let seg = Segment {
            segment: self.segment,
            text,
            start_secs: start,
            end_secs: end,
        };
        self.segment += 1;
        self.last_partial.clear();
        seg
    }
}

/// Faint deterministic noise the recognizer accepts as room tone.
pub fn preroll() -> Vec<f32> {
    let mut seed: u32 = 0x9E37_79B9;
    (0..PREROLL_SAMPLES)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            ((seed % 2001) as f32 - 1000.0) / 1000.0 * PREROLL_AMPLITUDE
        })
        .collect()
}

/// The streaming recognizer emits upper-case tokens for English models;
/// readers dictate prose, so hand back sentence case with the leading space
/// trimmed.
pub fn normalize(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let all_upper = trimmed
        .chars()
        .filter(|c| c.is_alphabetic())
        .all(|c| c.is_uppercase());
    if !all_upper {
        return trimmed.to_string();
    }
    let lower = trimmed.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut first = true;
    for ch in lower.chars() {
        if first && ch.is_alphabetic() {
            out.extend(ch.to_uppercase());
            first = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// Join committed segments into one readable string.
pub fn join_segments(segments: &[Segment]) -> String {
    segments
        .iter()
        .map(|s| s.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Where the second-pass model stands, for `voice::doctor`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FinalState {
    /// No second-pass model configured.
    Off,
    /// Named in the configuration but not on disk yet, and no download running.
    Missing,
    /// Downloading in the background; streaming text stands meanwhile.
    Downloading,
    /// On disk, loads on next use.
    Installed,
    /// Loaded and refining utterances.
    Loaded,
    /// The configured id is not in the catalog.
    Unknown,
}

/// Owns the loaded recognizers and reloads them when the configuration
/// changes.
pub struct Engine {
    loaded: Mutex<Option<Arc<Loaded>>>,
    final_loaded: Mutex<Option<Arc<FinalLoaded>>>,
    load_lock: Mutex<()>,
    final_download: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self {
            loaded: Mutex::new(None),
            final_loaded: Mutex::new(None),
            load_lock: Mutex::new(()),
            final_download: Mutex::new(None),
        }
    }

    /// The streaming recognizer if it is loaded for exactly this
    /// configuration.
    pub async fn loaded_for(&self, cfg: &WorkerConfig) -> Option<Arc<Loaded>> {
        let key = LoadKey::from_config(cfg);
        self.loaded
            .lock()
            .await
            .as_ref()
            .filter(|l| l.key == key)
            .cloned()
    }

    /// The second-pass recognizer if it is loaded for exactly this
    /// configuration.
    pub async fn final_loaded_for(&self, cfg: &WorkerConfig) -> Option<Arc<FinalLoaded>> {
        if cfg.stt.final_model.trim().is_empty() {
            return None;
        }
        let key = FinalKey::from_config(cfg);
        self.final_loaded
            .lock()
            .await
            .as_ref()
            .filter(|l| l.key == key)
            .cloned()
    }

    /// Whatever streaming recognizer is loaded, regardless of configuration.
    pub async fn current(&self) -> Option<Arc<Loaded>> {
        self.loaded.lock().await.clone()
    }

    /// Whatever second-pass recognizer is loaded, regardless of configuration.
    pub async fn current_final(&self) -> Option<Arc<FinalLoaded>> {
        self.final_loaded.lock().await.clone()
    }

    /// Drop the loaded recognizers so the next call reloads them.
    pub async fn invalidate(&self) {
        *self.loaded.lock().await = None;
        *self.final_loaded.lock().await = None;
    }

    /// Where the second-pass model stands right now.
    pub async fn final_state(&self, cfg: &WorkerConfig) -> FinalState {
        if cfg.stt.final_model.trim().is_empty() {
            return FinalState::Off;
        }
        let Some(spec) = models::find(&cfg.stt.final_model) else {
            return FinalState::Unknown;
        };
        if self.final_loaded_for(cfg).await.is_some() {
            return FinalState::Loaded;
        }
        if spec.is_installed(&cfg.models_path()) {
            return FinalState::Installed;
        }
        let downloading = self
            .final_download
            .lock()
            .await
            .as_ref()
            .is_some_and(|h| !h.is_finished());
        if downloading {
            FinalState::Downloading
        } else {
            FinalState::Missing
        }
    }

    /// Load the configured streaming model, downloading it first when it is
    /// missing. Concurrent callers wait for one load rather than each
    /// starting their own.
    pub async fn ensure_loaded(
        &self,
        cfg: &WorkerConfig,
        progress: Option<ProgressSink>,
    ) -> Result<Arc<Loaded>, String> {
        if let Some(loaded) = self.loaded_for(cfg).await {
            return Ok(loaded);
        }
        let _guard = self.load_lock.lock().await;
        if let Some(loaded) = self.loaded_for(cfg).await {
            return Ok(loaded);
        }
        let spec = models::find(&cfg.stt.model).ok_or_else(|| {
            format!(
                "unknown local model `{}`; voice::models::list names the choices",
                cfg.stt.model
            )
        })?;
        if spec.kind != ModelKind::StreamingTransducer {
            return Err(format!(
                "`{}` is a second-pass model; stt.model needs a streaming one",
                spec.id
            ));
        }
        let models_dir = cfg.models_path();
        if !spec.is_installed(&models_dir) {
            tracing::info!(model = spec.id, dir = %models_dir.display(), "downloading speech model");
            models::download(spec, &models_dir, progress).await?;
        }
        let key = LoadKey::from_config(cfg);
        let model_dir = spec.dir(&models_dir);
        let load_key = key.clone();
        let load_dir = model_dir.clone();
        let (recognizer, load_ms) = tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let config = OnlineRecognizerConfig {
                model_config: OnlineModelConfig {
                    transducer: OnlineTransducerModelConfig {
                        encoder: Some(load_dir.join(spec.encoder).to_string_lossy().into_owned()),
                        decoder: Some(load_dir.join(spec.decoder).to_string_lossy().into_owned()),
                        joiner: Some(load_dir.join(spec.joiner).to_string_lossy().into_owned()),
                    },
                    tokens: Some(load_dir.join(spec.tokens).to_string_lossy().into_owned()),
                    num_threads: load_key.num_threads as i32,
                    provider: Some("cpu".to_string()),
                    ..Default::default()
                },
                decoding_method: Some("greedy_search".to_string()),
                enable_endpoint: true,
                rule1_min_trailing_silence: load_key.rule1,
                rule2_min_trailing_silence: load_key.rule2,
                rule3_min_utterance_length: load_key.rule3,
                ..Default::default()
            };
            let recognizer = OnlineRecognizer::create(&config).ok_or_else(|| {
                format!("the recognizer refused the model in {}", load_dir.display())
            })?;
            Ok::<_, String>((recognizer, started.elapsed().as_millis()))
        })
        .await
        .map_err(|e| format!("model load task failed: {e}"))??;
        tracing::info!(model = spec.id, load_ms, "speech model loaded");
        let loaded = Arc::new(Loaded {
            recognizer,
            key,
            model_dir,
            load_ms,
        });
        *self.loaded.lock().await = Some(loaded.clone());
        Ok(loaded)
    }

    /// The second-pass recognizer when it can be had without waiting: loaded
    /// already, or on disk and loadable now. A missing model starts one
    /// background download and answers `None` until it lands, so dictation
    /// never blocks on hundreds of megabytes.
    pub async fn ensure_final_loaded(
        &self,
        cfg: &WorkerConfig,
        progress: Option<ProgressSink>,
    ) -> Result<Option<Arc<FinalLoaded>>, String> {
        if cfg.stt.final_model.trim().is_empty() {
            return Ok(None);
        }
        if let Some(loaded) = self.final_loaded_for(cfg).await {
            return Ok(Some(loaded));
        }
        let spec = models::find(&cfg.stt.final_model).ok_or_else(|| {
            format!(
                "unknown second-pass model `{}`; voice::models::list names the choices",
                cfg.stt.final_model
            )
        })?;
        if spec.kind != ModelKind::OfflineNemoTransducer {
            return Err(format!(
                "`{}` is a streaming model; stt.final_model needs a second-pass one",
                spec.id
            ));
        }
        let models_dir = cfg.models_path();
        if !spec.is_installed(&models_dir) {
            self.start_final_download(spec, models_dir, progress).await;
            return Ok(None);
        }
        let _guard = self.load_lock.lock().await;
        if let Some(loaded) = self.final_loaded_for(cfg).await {
            return Ok(Some(loaded));
        }
        let key = FinalKey::from_config(cfg);
        let load_key = key.clone();
        let load_dir = spec.dir(&models_dir);
        let (recognizer, load_ms) = tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let config = OfflineRecognizerConfig {
                model_config: OfflineModelConfig {
                    transducer: OfflineTransducerModelConfig {
                        encoder: Some(load_dir.join(spec.encoder).to_string_lossy().into_owned()),
                        decoder: Some(load_dir.join(spec.decoder).to_string_lossy().into_owned()),
                        joiner: Some(load_dir.join(spec.joiner).to_string_lossy().into_owned()),
                    },
                    tokens: Some(load_dir.join(spec.tokens).to_string_lossy().into_owned()),
                    num_threads: load_key.num_threads as i32,
                    provider: Some("cpu".to_string()),
                    model_type: Some("nemo_transducer".to_string()),
                    ..Default::default()
                },
                decoding_method: Some("greedy_search".to_string()),
                ..Default::default()
            };
            let recognizer = OfflineRecognizer::create(&config).ok_or_else(|| {
                format!("the recognizer refused the model in {}", load_dir.display())
            })?;
            Ok::<_, String>((recognizer, started.elapsed().as_millis()))
        })
        .await
        .map_err(|e| format!("model load task failed: {e}"))??;
        tracing::info!(model = spec.id, load_ms, "second-pass speech model loaded");
        let loaded = Arc::new(FinalLoaded {
            recognizer,
            key,
            load_ms,
        });
        *self.final_loaded.lock().await = Some(loaded.clone());
        Ok(Some(loaded))
    }

    async fn start_final_download(
        &self,
        spec: &'static ModelSpec,
        models_dir: PathBuf,
        progress: Option<ProgressSink>,
    ) {
        let mut slot = self.final_download.lock().await;
        if slot.as_ref().is_some_and(|h| !h.is_finished()) {
            return;
        }
        tracing::info!(model = spec.id, dir = %models_dir.display(), "downloading second-pass speech model in the background");
        *slot = Some(tokio::spawn(async move {
            match models::download(spec, &models_dir, progress).await {
                Ok(bytes) => {
                    tracing::info!(model = spec.id, bytes, "second-pass speech model installed")
                }
                Err(e) => {
                    tracing::warn!(model = spec.id, error = %e, "second-pass speech model download failed")
                }
            }
        }));
    }

    /// Transcribe a whole buffer on the configured backend: streaming pass
    /// for boundaries, second pass for the text when its model is present.
    pub async fn transcribe(
        &self,
        cfg: &WorkerConfig,
        samples: Vec<f32>,
        language: Option<&str>,
    ) -> Result<(Transcript, &'static str, String), String> {
        match cfg.stt.backend {
            SttBackend::Local => {
                let loaded = self.ensure_loaded(cfg, None).await?;
                let refiner = self.ensure_final_loaded(cfg, None).await?;
                let model = match &refiner {
                    Some(r) => r.key.model.clone(),
                    None => loaded.key.model.clone(),
                };
                let transcript = tokio::task::spawn_blocking(move || {
                    let transcript = loaded.transcribe(&samples);
                    match refiner {
                        Some(refiner) => refiner.refine_transcript(&samples, transcript),
                        None => transcript,
                    }
                })
                .await
                .map_err(|e| format!("transcription task failed: {e}"))?;
                Ok((transcript, "local", model))
            }
            SttBackend::Openai => {
                let transcript = remote_transcribe(cfg, &samples, language).await?;
                Ok((transcript, "openai", cfg.stt.openai.model.clone()))
            }
        }
    }
}

/// `POST {base_url}/audio/transcriptions`, multipart, `verbose_json` when the
/// server offers it and plain `json` otherwise.
pub async fn remote_transcribe(
    cfg: &WorkerConfig,
    samples: &[f32],
    language: Option<&str>,
) -> Result<Transcript, String> {
    let wav = audio::encode_wav(samples, TARGET_SAMPLE_RATE)?;
    let duration_secs = samples.len() as f32 / TARGET_SAMPLE_RATE as f32;
    let remote = &cfg.stt.openai;
    let url = format!(
        "{}/audio/transcriptions",
        remote.base_url.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .user_agent(concat!("iii-voice/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let mut form = reqwest::multipart::Form::new()
        .part(
            "file",
            reqwest::multipart::Part::bytes(wav)
                .file_name("audio.wav")
                .mime_str("audio/wav")
                .map_err(|e| format!("multipart: {e}"))?,
        )
        .text("model", remote.model.clone())
        .text("response_format", "verbose_json");
    let language = language
        .map(str::to_string)
        .filter(|l| !l.trim().is_empty())
        .or_else(|| (!remote.language.trim().is_empty()).then(|| remote.language.clone()));
    if let Some(language) = language {
        form = form.text("language", language);
    }
    let mut request = client.post(&url).multipart(form);
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
    let body = response
        .text()
        .await
        .map_err(|e| format!("read {url}: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "{url} answered {status}: {}",
            body.chars().take(300).collect::<String>()
        ));
    }
    parse_remote_transcript(&body, duration_secs)
}

fn parse_remote_transcript(body: &str, duration_secs: f32) -> Result<Transcript, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("transcription endpoint returned non-JSON: {e}"))?;
    let text = value
        .get("text")
        .and_then(|t| t.as_str())
        .map(|t| t.trim().to_string())
        .ok_or_else(|| "transcription endpoint returned no `text`".to_string())?;
    let mut segments = Vec::new();
    if let Some(items) = value.get("segments").and_then(|s| s.as_array()) {
        for (i, item) in items.iter().enumerate() {
            let seg_text = item
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or_default()
                .trim()
                .to_string();
            if seg_text.is_empty() {
                continue;
            }
            segments.push(Segment {
                segment: i as u32,
                text: seg_text,
                start_secs: item.get("start").and_then(|v| v.as_f64()).map(|v| v as f32),
                end_secs: item.get("end").and_then(|v| v.as_f64()).map(|v| v as f32),
            });
        }
    }
    if segments.is_empty() && !text.is_empty() {
        segments.push(Segment {
            segment: 0,
            text: text.clone(),
            start_secs: Some(0.0),
            end_secs: Some(duration_secs),
        });
    }
    Ok(Transcript {
        text,
        segments,
        duration_secs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_gives_sentence_case_to_shouting_models() {
        assert_eq!(normalize(" THE QUICK BROWN FOX"), "The quick brown fox");
        assert_eq!(normalize("Already fine"), "Already fine");
        assert_eq!(normalize("   "), "");
    }

    #[test]
    fn join_skips_empty_segments() {
        let segs = vec![
            Segment {
                segment: 0,
                text: "Hello".into(),
                start_secs: None,
                end_secs: None,
            },
            Segment {
                segment: 1,
                text: "  ".into(),
                start_secs: None,
                end_secs: None,
            },
            Segment {
                segment: 2,
                text: "world".into(),
                start_secs: None,
                end_secs: None,
            },
        ];
        assert_eq!(join_segments(&segs), "Hello world");
    }

    #[test]
    fn preroll_is_quiet_deterministic_noise() {
        let a = preroll();
        let b = preroll();
        assert_eq!(a, b);
        assert_eq!(a.len(), PREROLL_SAMPLES);
        assert!(a.iter().all(|s| s.abs() <= PREROLL_AMPLITUDE));
        assert!(a.iter().any(|s| *s != 0.0));
    }

    #[test]
    fn remote_verbose_json_keeps_segments() {
        let body = r#"{"text":" Hello there.","segments":[{"start":0.0,"end":1.2,"text":" Hello"},{"start":1.2,"end":2.0,"text":" there."}]}"#;
        let t = parse_remote_transcript(body, 2.0).unwrap();
        assert_eq!(t.text, "Hello there.");
        assert_eq!(t.segments.len(), 2);
        assert_eq!(t.segments[1].start_secs, Some(1.2));
    }

    #[test]
    fn remote_plain_json_becomes_one_segment() {
        let t = parse_remote_transcript(r#"{"text":"Just text"}"#, 3.5).unwrap();
        assert_eq!(t.segments.len(), 1);
        assert_eq!(t.segments[0].end_secs, Some(3.5));
        assert!(parse_remote_transcript(r#"{"nope":1}"#, 1.0).is_err());
    }

    #[test]
    fn load_keys_track_the_reloadable_fields() {
        let mut cfg = WorkerConfig::default();
        let a = LoadKey::from_config(&cfg);
        let f = FinalKey::from_config(&cfg);
        cfg.stt.silence_after_speech_secs = 1.5;
        assert_ne!(a, LoadKey::from_config(&cfg));
        assert_eq!(f, FinalKey::from_config(&cfg));
        cfg.stt.final_model = String::new();
        assert_ne!(f, FinalKey::from_config(&cfg));
    }

    #[tokio::test]
    async fn final_state_reports_off_and_unknown() {
        let engine = Engine::new();
        let mut cfg = WorkerConfig::default();
        cfg.stt.final_model = String::new();
        assert_eq!(engine.final_state(&cfg).await, FinalState::Off);
        cfg.stt.final_model = "nope".into();
        assert_eq!(engine.final_state(&cfg).await, FinalState::Unknown);
        assert!(engine.ensure_final_loaded(&cfg, None).await.is_err());
    }
}
