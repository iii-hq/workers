//! `voice::models::*` — the local model catalog and its downloads.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::AppState;
use crate::events::EventKind;
use crate::models::{self, Progress, ProgressSink};

pub const LIST_ID: &str = "voice::models::list";
pub const LIST_DESC: &str =
    "List the speech models the local backend can run, which one is active, \
                             and whether each is installed under models_dir.";

pub const DOWNLOAD_ID: &str = "voice::models::download";
pub const DOWNLOAD_DESC: &str =
    "Download a local speech model (the active one when id is omitted), \
                                 verifying every file's checksum. Progress arrives on the \
                                 voice::model-progress trigger.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListRequest {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    /// `streaming_transducer` (live partials) or `offline_nemo_transducer`
    /// (second pass).
    pub kind: crate::models::ModelKind,
    pub languages: Vec<String>,
    pub license: String,
    pub size_bytes: u64,
    pub installed: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResponse {
    /// The model the configuration names.
    pub active: String,
    pub models_dir: String,
    pub models: Vec<ModelEntry>,
}

pub async fn list(state: &AppState, _req: ListRequest) -> Result<ListResponse, String> {
    let cfg = state.cfg.read().await.clone();
    let dir = cfg.models_path();
    Ok(ListResponse {
        active: cfg.stt.model.clone(),
        models_dir: dir.to_string_lossy().into_owned(),
        models: models::catalog()
            .iter()
            .map(|m| ModelEntry {
                id: m.id.to_string(),
                name: m.name.to_string(),
                kind: m.kind,
                languages: m.languages.iter().map(|l| l.to_string()).collect(),
                license: m.license.to_string(),
                size_bytes: m.size_bytes(),
                installed: m.is_installed(&dir),
            })
            .collect(),
    })
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct DownloadRequest {
    /// Model id from voice::models::list; omit for the active one.
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DownloadResponse {
    pub id: String,
    pub installed: bool,
    /// Bytes written by this call (0 when everything was already present).
    pub bytes: u64,
}

pub async fn download(state: &AppState, req: DownloadRequest) -> Result<DownloadResponse, String> {
    let cfg = state.cfg.read().await.clone();
    let id = req.id.unwrap_or_else(|| cfg.stt.model.clone());
    let spec = models::find(&id)
        .ok_or_else(|| format!("unknown model `{id}`; voice::models::list names the choices"))?;
    let dir = cfg.models_path();
    let bytes = models::download(spec, &dir, Some(progress_sink(state))).await?;
    Ok(DownloadResponse {
        id: spec.id.to_string(),
        installed: spec.is_installed(&dir),
        bytes,
    })
}

/// A sink that fans download progress out on `voice::model-progress`.
pub fn progress_sink(state: &AppState) -> ProgressSink {
    let emitter = state.emitter.clone();
    Arc::new(move |progress: Progress| {
        let emitter = emitter.clone();
        tokio::spawn(async move {
            emitter
                .emit(EventKind::ModelProgress, None, &progress)
                .await;
        });
    })
}
