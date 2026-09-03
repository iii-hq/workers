//! `voice::doctor` — what the worker can do right now.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::AppState;
use crate::config::{SttBackend, TtsBackend};
use crate::engine::FinalState;
use crate::models;
use crate::tts;

pub const ID: &str = "voice::doctor";
pub const DESC: &str = "Report the speech-to-text backend and whether its model is installed and \
                        loaded, the read-aloud backend and whether its command exists, and how many \
                        dictation sessions are open.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct Request {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SttReport {
    /// `local` or `openai`.
    pub backend: String,
    pub model: String,
    pub installed: bool,
    pub loaded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_ms: Option<u64>,
    pub models_dir: String,
    /// Set when the configured model id is not in the catalog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// The second-pass model that gives final text its punctuation and
    /// accuracy, and where it stands.
    pub final_model: String,
    pub final_state: FinalState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_load_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TtsReport {
    /// `host`, `openai`, or `off`.
    pub backend: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub available: bool,
    /// Host playbacks currently running.
    pub playing: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    pub stt: SttReport,
    pub tts: TtsReport,
    pub sessions: usize,
    pub version: String,
}

pub async fn handle(state: &AppState, _req: Request) -> Result<Response, String> {
    let cfg = state.cfg.read().await.clone();
    let dir = cfg.models_path();
    let spec = models::find(&cfg.stt.model);
    let loaded = state.engine.loaded_for(&cfg).await;
    let stt = match cfg.stt.backend {
        SttBackend::Local => SttReport {
            backend: "local".into(),
            model: cfg.stt.model.clone(),
            installed: spec.is_some_and(|s| s.is_installed(&dir)),
            loaded: loaded.is_some(),
            load_ms: loaded.as_ref().map(|l| l.load_ms as u64),
            models_dir: dir.to_string_lossy().into_owned(),
            problem: spec
                .is_none()
                .then(|| format!("unknown model `{}`", cfg.stt.model)),
            final_model: cfg.stt.final_model.clone(),
            final_state: state.engine.final_state(&cfg).await,
            final_load_ms: state
                .engine
                .final_loaded_for(&cfg)
                .await
                .map(|l| l.load_ms as u64),
        },
        SttBackend::Router => SttReport {
            backend: "router".into(),
            model: if cfg.stt.router.model.trim().is_empty() {
                "router picks".into()
            } else {
                cfg.stt.router.model.clone()
            },
            installed: true,
            loaded: true,
            load_ms: None,
            models_dir: dir.to_string_lossy().into_owned(),
            problem: crate::router::problem(&state.iii, "stt").await,
            final_model: String::new(),
            final_state: FinalState::Off,
            final_load_ms: None,
        },
        SttBackend::Openai => SttReport {
            backend: "openai".into(),
            model: cfg.stt.openai.model.clone(),
            installed: true,
            loaded: true,
            load_ms: None,
            models_dir: dir.to_string_lossy().into_owned(),
            problem: cfg
                .stt
                .openai
                .base_url
                .trim()
                .is_empty()
                .then(|| "stt.openai.base_url is empty".to_string()),
            final_model: String::new(),
            final_state: FinalState::Off,
            final_load_ms: None,
        },
    };
    let tts = match cfg.tts.backend {
        TtsBackend::Host => {
            let command = tts::host_command();
            TtsReport {
                backend: "host".into(),
                command: command.as_ref().map(|c| c.program.to_string()),
                available: command.is_some(),
                playing: state.speaker.playing().await,
            }
        }
        TtsBackend::Openai => TtsReport {
            backend: "openai".into(),
            command: None,
            available: !cfg.tts.openai.base_url.trim().is_empty(),
            playing: 0,
        },
        TtsBackend::Router => TtsReport {
            backend: "router".into(),
            command: crate::router::problem(&state.iii, "tts").await,
            available: crate::router::problem(&state.iii, "tts").await.is_none(),
            playing: 0,
        },
        TtsBackend::Off => TtsReport {
            backend: "off".into(),
            command: None,
            available: false,
            playing: 0,
        },
    };
    Ok(Response {
        stt,
        tts,
        sessions: state.sessions.count().await,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}
