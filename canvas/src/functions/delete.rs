//! `canvas::delete` — remove a stored canvas.
//!
//! Removes both the record and its side-index row, so a deleted canvas
//! disappears from `canvas::list` in the same call.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;
use crate::store::Store;

pub const ID: &str = "canvas::delete";
pub const DESC: &str = "Delete a stored canvas by its stable 8-character id. Deleting an unknown \
                        id is not an error: the response reports deleted=false instead.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// Stable 8-character canvas id.
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// The id the call asked to delete.
    pub id: String,

    /// `true` when a record existed and was removed; `false` for an unknown
    /// id.
    pub deleted: bool,
}

pub async fn handle(store: &Store, req: Request, _cfg: &WorkerConfig) -> Result<Response, String> {
    let deleted = store.delete(&req.id).await?;
    Ok(Response {
        id: req.id,
        deleted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functions::{create, get, list};
    use crate::store::CanvasFormat;

    /// Index consistency after delete: the record is gone, `canvas::get`
    /// errors, and `canvas::list` no longer shows the id.
    #[tokio::test]
    async fn delete_removes_the_record_from_get_and_list() {
        let store = Store::in_memory();
        let cfg = WorkerConfig::default();
        let keep = create::handle(
            &store,
            create::Request {
                name: Some("keep".into()),
                format: Some(CanvasFormat::Mermaid),
                source: "flowchart TD\n  A --> B\n".into(),
            },
            &cfg,
        )
        .await
        .expect("creates");
        let doomed = create::handle(
            &store,
            create::Request {
                name: Some("doomed".into()),
                format: Some(CanvasFormat::Mermaid),
                source: "mindmap\n  root((x))\n".into(),
            },
            &cfg,
        )
        .await
        .expect("creates");

        let out = handle(
            &store,
            Request {
                id: doomed.id.clone(),
            },
            &cfg,
        )
        .await
        .expect("deletes");
        assert_eq!(out.id, doomed.id);
        assert!(out.deleted);

        get::handle(
            &store,
            get::Request {
                id: doomed.id.clone(),
            },
            &cfg,
        )
        .await
        .expect_err("deleted id no longer gets");

        let listed = list::handle(&store, list::Request::default(), &cfg)
            .await
            .expect("lists");
        assert_eq!(listed.count, 1);
        assert_eq!(listed.canvases[0].id, keep.id);
    }

    #[tokio::test]
    async fn deleting_an_unknown_id_reports_false_without_erroring() {
        let store = Store::in_memory();
        let out = handle(
            &store,
            Request {
                id: "zzzz9999".into(),
            },
            &WorkerConfig::default(),
        )
        .await
        .expect("answers");
        assert_eq!(out.id, "zzzz9999");
        assert!(!out.deleted);
    }
}
