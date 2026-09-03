//! Dictation sessions: one open recognizer stream per caller, fed by
//! `voice::dictation::push`, delivering transcript events straight to the
//! caller's function and to every `voice::transcript` subscriber.
//!
//! Each session keeps the audio of the utterance in progress. When the
//! streaming recognizer ends an utterance, that audio goes through the
//! second-pass model (when it is loaded) and the `final` event carries the
//! refined text; the streaming text stands otherwise. Events carry a
//! per-session sequence number so a receiver can drop anything it already
//! saw. A session that goes quiet for `session_idle_secs` is closed by the
//! sweep with reason `idle`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iii_sdk::IIIClient;
use tokio::sync::Mutex;

use crate::audio::{self, TARGET_SAMPLE_RATE};
use crate::config::{SttBackend, WorkerConfig};
use crate::configuration::ConfigCell;
use crate::engine::{self, Engine, FinalLoaded, Segment, Stream, Transcript};
use crate::events::{
    Emitter, EventKind, SessionStartedEvent, SessionStoppedEvent, TranscriptEvent, TranscriptKind,
};
use crate::models::ProgressSink;

/// Bytes accepted per push, decoded. 64 KiB is two seconds of 16 kHz PCM.
pub const MAX_PUSH_BYTES: usize = 64 * 1024;
/// Audio kept for the utterance in progress, in samples: past this the
/// oldest audio is dropped, so a runaway utterance cannot grow memory.
const MAX_UTTERANCE_SAMPLES: usize = TARGET_SAMPLE_RATE as usize * 60;
/// Audio kept from before the streaming recognizer's utterance start, so the
/// second pass hears the onset of the first word.
const ONSET_PAD_SAMPLES: usize = 8_000;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Where a finished utterance goes for its final text. Live words always
/// come from the local streaming model; the second pass follows the
/// configured engine, so the model a person picked is the one that writes
/// the transcript.
pub enum Refiner {
    Local(Arc<FinalLoaded>),
    Remote {
        iii: Arc<IIIClient>,
        cfg: Arc<WorkerConfig>,
    },
}

/// A remote second pass that has not answered by then keeps the streaming
/// text; dictation must not stall on a slow provider.
const REMOTE_REFINE_TIMEOUT_MS: u64 = 30_000;

impl Refiner {
    async fn refine(&self, audio: Vec<f32>) -> Result<String, String> {
        match self {
            Refiner::Local(loaded) => {
                let loaded = loaded.clone();
                tokio::task::spawn_blocking(move || loaded.refine(&audio))
                    .await
                    .map_err(|e| format!("second pass task failed: {e}"))
            }
            Refiner::Remote { iii, cfg } => match cfg.stt.backend {
                SttBackend::Router => crate::router::transcribe_within(
                    iii,
                    cfg,
                    &audio,
                    None,
                    REMOTE_REFINE_TIMEOUT_MS,
                )
                .await
                .map(|(transcript, _)| transcript.text),
                SttBackend::Openai => engine::remote_transcribe(cfg, &audio, None)
                    .await
                    .map(|transcript| transcript.text),
                SttBackend::Local => Ok(String::new()),
            },
        }
    }

    fn label(&self) -> String {
        match self {
            Refiner::Local(loaded) => loaded.key.model.clone(),
            Refiner::Remote { cfg, .. } => match cfg.stt.backend {
                SttBackend::Router if cfg.stt.router.model.trim().is_empty() => "router".into(),
                SttBackend::Router => cfg.stt.router.model.trim().to_string(),
                SttBackend::Openai => cfg.stt.openai.model.clone(),
                SttBackend::Local => String::new(),
            },
        }
    }
}

pub struct Session {
    pub id: String,
    pub output_function_id: String,
    pub model: String,
    stream: Stream,
    refiner: Option<Arc<Refiner>>,
    seq: u64,
    last_push_seq: Option<u64>,
    segments: Vec<Segment>,
    partial: String,
    /// Audio since the last committed segment, for the second pass.
    utterance: Vec<f32>,
    pub started_at: Instant,
    last_audio: Instant,
}

impl Session {
    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn event(
        &mut self,
        kind: TranscriptKind,
        text: String,
        reason: Option<String>,
    ) -> TranscriptEvent {
        TranscriptEvent {
            session_id: self.id.clone(),
            seq: self.next_seq(),
            kind,
            text,
            segment: self.stream.segment(),
            timestamp_ms: now_ms(),
            reason,
        }
    }

    pub fn idle_secs(&self) -> u64 {
        self.last_audio.elapsed().as_secs()
    }

    pub fn duration_secs(&self) -> f32 {
        self.stream.fed_secs()
    }

    /// Keep the utterance buffer bounded.
    fn buffer(&mut self, samples: &[f32]) {
        self.utterance.extend_from_slice(samples);
        if self.utterance.len() > MAX_UTTERANCE_SAMPLES {
            let excess = self.utterance.len() - MAX_UTTERANCE_SAMPLES;
            self.utterance.drain(..excess);
        }
    }

    /// Commit one streaming segment: the second pass replaces its text when
    /// the model is loaded and heard something; the utterance buffer keeps
    /// only a short tail so the next utterance's onset survives.
    async fn commit(&mut self, mut seg: Segment) -> TranscriptEvent {
        if let Some(refiner) = self.refiner.clone() {
            let audio = self.utterance.clone();
            match refiner.refine(audio).await {
                Ok(refined) if !refined.trim().is_empty() => seg.text = refined.trim().to_string(),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "second pass failed; streaming text stands"),
            }
        }
        let keep = self.utterance.len().min(ONSET_PAD_SAMPLES);
        let tail = self.utterance.split_off(self.utterance.len() - keep);
        self.utterance = tail;
        self.partial.clear();
        let text = seg.text.clone();
        self.segments.push(seg);
        self.event(TranscriptKind::Final, text, None)
    }

    fn transcript(&self) -> Transcript {
        let mut segments = self.segments.clone();
        if !self.partial.trim().is_empty() {
            segments.push(Segment {
                segment: self.stream.segment(),
                text: self.partial.clone(),
                start_secs: None,
                end_secs: None,
            });
        }
        Transcript {
            text: engine::join_segments(&segments),
            segments,
            duration_secs: self.stream.fed_secs(),
        }
    }
}

/// Summary of an open session for `voice::dictation::list`.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct SessionSummary {
    pub session_id: String,
    pub model: String,
    pub started_at_ms: i64,
    pub duration_secs: f32,
    pub segments: u32,
    pub idle_secs: u64,
}

pub struct Sessions {
    inner: Mutex<HashMap<String, Arc<Mutex<Session>>>>,
    iii: Arc<IIIClient>,
    engine: Arc<Engine>,
    emitter: Arc<Emitter>,
    cfg: ConfigCell,
    progress: Mutex<Option<ProgressSink>>,
}

impl Sessions {
    pub fn new(
        iii: Arc<IIIClient>,
        engine: Arc<Engine>,
        emitter: Arc<Emitter>,
        cfg: ConfigCell,
    ) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            iii,
            engine,
            emitter,
            cfg,
            progress: Mutex::new(None),
        }
    }

    /// Where background model downloads report progress.
    pub async fn set_progress_sink(&self, sink: ProgressSink) {
        *self.progress.lock().await = Some(sink);
    }

    pub async fn config(&self) -> Arc<WorkerConfig> {
        self.cfg.read().await.clone()
    }

    pub async fn count(&self) -> usize {
        self.inner.lock().await.len()
    }

    async fn get(&self, id: &str) -> Result<Arc<Mutex<Session>>, String> {
        self.inner.lock().await.get(id).cloned().ok_or_else(|| {
            format!("no dictation session `{id}`; start one with voice::dictation::start")
        })
    }

    /// Open a session and prime its stream.
    pub async fn start(&self, output_function_id: String) -> Result<Arc<Mutex<Session>>, String> {
        validate_output_function_id(&output_function_id)?;
        let cfg = self.config().await;
        if self.count().await >= cfg.max_sessions {
            return Err(format!(
                "{} dictation sessions are already open (max_sessions); stop one first",
                cfg.max_sessions
            ));
        }
        let progress = self.progress.lock().await.clone();
        let loaded = self.engine.ensure_loaded(&cfg, progress.clone()).await?;
        let refiner = match cfg.stt.backend {
            SttBackend::Local => match self.engine.ensure_final_loaded(&cfg, progress).await {
                Ok(refiner) => refiner.map(Refiner::Local),
                Err(e) => {
                    tracing::warn!(error = %e, "second-pass model unavailable; streaming text stands");
                    None
                }
            },
            SttBackend::Router | SttBackend::Openai => Some(Refiner::Remote {
                iii: self.iii.clone(),
                cfg: cfg.clone(),
            }),
        };
        let refiner = refiner.map(Arc::new);
        let id = format!("d_{}", uuid::Uuid::new_v4().simple());
        let model = match &refiner {
            Some(r) => format!("{}+{}", loaded.key.model, r.label()),
            None => loaded.key.model.clone(),
        };
        tracing::info!(session_id = %id, model = %model, output = %output_function_id, "dictation session started");
        let session = Session {
            id: id.clone(),
            output_function_id,
            model: model.clone(),
            stream: loaded.open_stream(),
            refiner,
            seq: 0,
            last_push_seq: None,
            segments: Vec::new(),
            partial: String::new(),
            utterance: Vec::new(),
            started_at: Instant::now(),
            last_audio: Instant::now(),
        };
        let handle = Arc::new(Mutex::new(session));
        {
            let mut inner = self.inner.lock().await;
            if inner.len() >= cfg.max_sessions {
                return Err(format!(
                    "{} dictation sessions are already open (max_sessions); stop one first",
                    cfg.max_sessions
                ));
            }
            inner.insert(id.clone(), handle.clone());
        }
        self.emitter
            .emit(
                EventKind::SessionStarted,
                Some(&id),
                &SessionStartedEvent {
                    session_id: id.clone(),
                    model,
                    timestamp_ms: now_ms(),
                },
            )
            .await;
        Ok(handle)
    }

    /// Feed one base64 PCM chunk. Returns the milliseconds of audio it held.
    pub async fn push(&self, id: &str, seq: u64, pcm16_base64: &str) -> Result<u32, String> {
        let handle = self.get(id).await?;
        let samples = audio::decode_pcm16_base64(pcm16_base64, MAX_PUSH_BYTES)?;
        let mut session = handle.lock().await;
        if let Some(last) = session.last_push_seq {
            if seq <= last {
                return Err(format!(
                    "chunk seq {seq} is not after the last accepted seq {last}"
                ));
            }
        }
        session.last_push_seq = Some(seq);
        session.last_audio = Instant::now();
        session.buffer(&samples);
        let step = session.stream.feed(&samples);
        let mut events = Vec::new();
        for seg in step.finals {
            events.push(session.commit(seg).await);
        }
        if let Some(partial) = step.partial {
            session.partial = partial.clone();
            events.push(session.event(TranscriptKind::Partial, partial, None));
        }
        let output = session.output_function_id.clone();
        drop(session);
        for event in events {
            self.deliver(&output, &event).await;
        }
        Ok(((samples.len() as f32 / TARGET_SAMPLE_RATE as f32) * 1000.0) as u32)
    }

    /// Close a session, flushing the recognizer unless `discard`.
    pub async fn stop(&self, id: &str, discard: bool, reason: &str) -> Result<Transcript, String> {
        let handle = self
            .inner
            .lock()
            .await
            .remove(id)
            .ok_or_else(|| format!("no dictation session `{id}`"))?;
        let mut session = handle.lock().await;
        let mut events = Vec::new();
        if !discard {
            for seg in session.stream.finish() {
                events.push(session.commit(seg).await);
            }
        } else {
            session.segments.clear();
            session.partial.clear();
        }
        let transcript = session.transcript();
        tracing::info!(
            session_id = %id,
            reason,
            audio_secs = transcript.duration_secs,
            chunks = session.last_push_seq.unwrap_or(0),
            segments = transcript.segments.len(),
            chars = transcript.text.len(),
            "dictation session closed"
        );
        events.push(session.event(
            TranscriptKind::Closed,
            transcript.text.clone(),
            Some(reason.to_string()),
        ));
        let output = session.output_function_id.clone();
        drop(session);
        for event in events {
            self.deliver(&output, &event).await;
        }
        self.emitter
            .emit(
                EventKind::SessionStopped,
                Some(id),
                &SessionStoppedEvent {
                    session_id: id.to_string(),
                    reason: reason.to_string(),
                    timestamp_ms: now_ms(),
                },
            )
            .await;
        Ok(transcript)
    }

    pub async fn list(&self) -> Vec<SessionSummary> {
        let handles: Vec<Arc<Mutex<Session>>> = self.inner.lock().await.values().cloned().collect();
        let mut out = Vec::with_capacity(handles.len());
        for handle in handles {
            let s = handle.lock().await;
            out.push(SessionSummary {
                session_id: s.id.clone(),
                model: s.model.clone(),
                started_at_ms: now_ms() - s.started_at.elapsed().as_millis() as i64,
                duration_secs: s.duration_secs(),
                segments: s.stream.segment(),
                idle_secs: s.idle_secs(),
            });
        }
        out.sort_by_key(|s| s.started_at_ms);
        out
    }

    /// Close every session idle past the configured limit.
    pub async fn sweep_idle(&self) {
        let idle_limit = self.config().await.session_idle_secs;
        let handles: Vec<(String, Arc<Mutex<Session>>)> = self
            .inner
            .lock()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (id, handle) in handles {
            let idle = handle.lock().await.idle_secs();
            if idle >= idle_limit {
                tracing::info!(session_id = %id, idle, "closing idle dictation session");
                let _ = self.stop(&id, false, "idle").await;
            }
        }
    }

    /// Close everything on worker shutdown so subscribers see the end.
    pub async fn stop_all(&self) {
        let ids: Vec<String> = self.inner.lock().await.keys().cloned().collect();
        for id in ids {
            let _ = self.stop(&id, false, "shutdown").await;
        }
    }

    async fn deliver(&self, output_function_id: &str, event: &TranscriptEvent) {
        self.emitter.deliver_direct(output_function_id, event).await;
        self.emitter
            .emit(EventKind::Transcript, Some(&event.session_id), event)
            .await;
    }

    /// Spawn the idle sweep.
    pub fn spawn_sweep(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let sessions = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(10));
            loop {
                tick.tick().await;
                sessions.sweep_idle().await;
            }
        })
    }
}

/// A caller-supplied delivery target must look like a function id: no
/// whitespace, no empty parts, and not one of this worker's own functions
/// (which would loop transcript events back into the worker).
pub fn validate_output_function_id(id: &str) -> Result<(), String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(
            "output_function_id is required: the function that receives transcript events"
                .to_string(),
        );
    }
    if trimmed.len() > 512 || trimmed.chars().any(char::is_whitespace) {
        return Err(
            "output_function_id must be a single function id without whitespace".to_string(),
        );
    }
    if trimmed.starts_with("voice::") {
        return Err(
            "output_function_id must not be one of the voice worker's own functions".to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_function_ids_are_checked() {
        assert!(validate_output_function_id("iii::voice-ui::transcript::console-1").is_ok());
        assert!(validate_output_function_id("").is_err());
        assert!(validate_output_function_id("has space").is_err());
        assert!(validate_output_function_id("voice::dictation::push").is_err());
    }
}
