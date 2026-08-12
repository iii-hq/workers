//! `canvas::update` — edit a stored canvas.
//!
//! The id is stable across updates; `updated_at` moves, and for mermaid the
//! family is re-derived when the source changes. The format is fixed at
//! create time — a mermaid canvas never silently becomes a whiteboard.

use schemars::JsonSchema;
use serde::Deserialize;

use crate::config::WorkerConfig;
use crate::functions::{create, family};
use crate::store::{CanvasFormat, CanvasRecord, Store};

pub const ID: &str = "canvas::update";
pub const DESC: &str = "Update a stored canvas's name and/or source by id. The id never changes; \
                        updated_at is stamped and, for mermaid, the diagram family is re-derived \
                        from the new source. Returns the full updated record.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// Stable 8-character canvas id.
    pub id: String,

    /// New canvas name. Omit to keep the current one.
    #[serde(default)]
    pub name: Option<String>,

    /// New source (mermaid text or excalidraw scene JSON, matching the
    /// canvas's format). Omit to keep the current one.
    #[serde(default)]
    pub source: Option<String>,
}

/// The full updated record.
pub type Response = CanvasRecord;

pub async fn handle(store: &Store, req: Request, cfg: &WorkerConfig) -> Result<Response, String> {
    if req.name.is_none() && req.source.is_none() {
        return Err("nothing to update — pass name and/or source".to_string());
    }
    let mut record = store.load(&req.id).await?.ok_or_else(|| {
        format!(
            "canvas '{}' not found — canvas::list shows the stored ids",
            req.id
        )
    })?;

    if let Some(name) = req.name {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("name cannot be blank — omit it to keep the current name".to_string());
        }
        record.name = trimmed.to_string();
    }
    if let Some(source) = req.source {
        create::check_source(&source, record.format, cfg)?;
        if record.format == CanvasFormat::Mermaid {
            record.family = family::detect(&source);
        }
        record.source = source;
    }
    record.updated_at = create::unix_now();

    store.save(&record).await?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed(store: &Store) -> CanvasRecord {
        create::handle(
            store,
            create::Request {
                name: Some("flow".into()),
                format: Some(CanvasFormat::Mermaid),
                source: "flowchart TD\n  A --> B\n".into(),
            },
            &WorkerConfig::default(),
        )
        .await
        .expect("seeds")
    }

    /// The property callers chain on: the id survives every update, and
    /// `created_at` stays put while `updated_at` moves forward.
    #[tokio::test]
    async fn the_id_is_stable_and_the_family_follows_the_source() {
        let store = Store::in_memory();
        let cfg = WorkerConfig::default();
        let created = seed(&store).await;

        let updated = handle(
            &store,
            Request {
                id: created.id.clone(),
                name: Some("as a sequence".into()),
                source: Some("sequenceDiagram\n  A->>B: hi\n".into()),
            },
            &cfg,
        )
        .await
        .expect("updates");

        assert_eq!(updated.id, created.id);
        assert_eq!(updated.created_at, created.created_at);
        assert!(updated.updated_at >= created.updated_at);
        assert_eq!(updated.name, "as a sequence");
        assert_eq!(updated.family.as_deref(), Some("sequenceDiagram"));

        let stored = store
            .load(&created.id)
            .await
            .expect("load")
            .expect("stored");
        assert_eq!(stored, updated);
    }

    #[tokio::test]
    async fn a_name_only_update_keeps_the_source_and_family() {
        let store = Store::in_memory();
        let cfg = WorkerConfig::default();
        let created = seed(&store).await;

        let updated = handle(
            &store,
            Request {
                id: created.id.clone(),
                name: Some("renamed".into()),
                source: None,
            },
            &cfg,
        )
        .await
        .expect("updates");
        assert_eq!(updated.name, "renamed");
        assert_eq!(updated.source, created.source);
        assert_eq!(updated.family, created.family);
    }

    #[tokio::test]
    async fn unknown_ids_empty_updates_and_blank_names_are_rejected() {
        let store = Store::in_memory();
        let cfg = WorkerConfig::default();
        let created = seed(&store).await;

        let missing = handle(
            &store,
            Request {
                id: "zzzz9999".into(),
                name: Some("x".into()),
                source: None,
            },
            &cfg,
        )
        .await
        .expect_err("unknown id");
        assert!(missing.contains("zzzz9999"), "{missing}");

        let nothing = handle(
            &store,
            Request {
                id: created.id.clone(),
                name: None,
                source: None,
            },
            &cfg,
        )
        .await
        .expect_err("no fields");
        assert!(nothing.contains("nothing to update"), "{nothing}");

        let blank = handle(
            &store,
            Request {
                id: created.id.clone(),
                name: Some("  ".into()),
                source: None,
            },
            &cfg,
        )
        .await
        .expect_err("blank name");
        assert!(blank.contains("blank"), "{blank}");
    }

    /// A source that no longer names a family downgrades to `family: null`
    /// rather than keeping a stale label.
    #[tokio::test]
    async fn a_headerless_source_clears_the_family() {
        let store = Store::in_memory();
        let created = seed(&store).await;
        let updated = handle(
            &store,
            Request {
                id: created.id.clone(),
                name: None,
                source: Some("just notes, not a diagram\n".into()),
            },
            &WorkerConfig::default(),
        )
        .await
        .expect("updates");
        assert_eq!(updated.family, None);
    }
}
