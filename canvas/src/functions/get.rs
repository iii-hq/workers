//! `canvas::get` — read one canvas by id.

use schemars::JsonSchema;
use serde::Deserialize;

use crate::config::WorkerConfig;
use crate::store::{CanvasRecord, Store};

pub const ID: &str = "canvas::get";
pub const DESC: &str = "Read one canvas by its stable 8-character id. Returns the full stored \
                        record, including the editable source. Errors when the id is unknown.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// Stable 8-character canvas id, as returned by `canvas::create` and
    /// `canvas::list`.
    pub id: String,
}

/// The full stored record.
pub type Response = CanvasRecord;

pub async fn handle(store: &Store, req: Request, _cfg: &WorkerConfig) -> Result<Response, String> {
    store.load(&req.id).await?.ok_or_else(|| {
        format!(
            "canvas '{}' not found — canvas::list shows the stored ids",
            req.id
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functions::create;
    use crate::store::CanvasFormat;

    #[tokio::test]
    async fn get_returns_the_stored_record() {
        let store = Store::in_memory();
        let cfg = WorkerConfig::default();
        let created = create::handle(
            &store,
            create::Request {
                name: Some("flow".into()),
                format: Some(CanvasFormat::Mermaid),
                source: "flowchart TD\n  A --> B\n".into(),
            },
            &cfg,
        )
        .await
        .expect("creates");

        let got = handle(
            &store,
            Request {
                id: created.id.clone(),
            },
            &cfg,
        )
        .await
        .expect("gets");
        assert_eq!(got, created);
    }

    /// The error must name the id the caller asked for — an agent retries on
    /// the message text alone.
    #[tokio::test]
    async fn an_unknown_id_errors_by_name() {
        let store = Store::in_memory();
        let err = handle(
            &store,
            Request {
                id: "zzzz9999".into(),
            },
            &WorkerConfig::default(),
        )
        .await
        .expect_err("unknown id");
        assert!(err.contains("zzzz9999"), "{err}");
    }
}
