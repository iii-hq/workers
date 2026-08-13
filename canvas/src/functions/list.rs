//! `canvas::list` — every stored canvas, newest first.
//!
//! Ordering and filtering run over the side index (`index` state key), so the
//! cost of listing never grows with the size of the stored sources; only the
//! records that survive the filter and the `max_list` cap are actually
//! loaded.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;
use crate::store::{CanvasFormat, CanvasRecord, Store};

pub const ID: &str = "canvas::list";
pub const DESC: &str = "List stored canvases, newest first, optionally filtered by format. Each \
                        entry is the full record including its source; the response is capped by \
                        the configured max_list.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct Request {
    /// Only return canvases of this format. Omit for every canvas.
    #[serde(default)]
    pub format: Option<CanvasFormat>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// The stored records, newest first.
    pub canvases: Vec<CanvasRecord>,

    /// How many records this response carries.
    pub count: usize,
}

pub async fn handle(store: &Store, req: Request, cfg: &WorkerConfig) -> Result<Response, String> {
    let index = store.index().await?;
    let mut canvases = Vec::new();
    for entry in index {
        if canvases.len() >= cfg.max_list {
            break;
        }
        if let Some(format) = req.format {
            if entry.format != format {
                continue;
            }
        }
        // An index row whose record vanished (an interrupted delete) is
        // skipped rather than failing the whole listing.
        if let Some(record) = store.load(&entry.id).await? {
            canvases.push(record);
        }
    }
    let count = canvases.len();
    Ok(Response { canvases, count })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functions::create;

    async fn seed(store: &Store, name: &str, format: CanvasFormat, source: &str) -> CanvasRecord {
        create::handle(
            store,
            create::Request {
                name: Some(name.into()),
                format: Some(format),
                source: source.into(),
            },
            &WorkerConfig::default(),
        )
        .await
        .expect("seeds")
    }

    #[tokio::test]
    async fn list_returns_full_records_newest_first() {
        let store = Store::in_memory();
        let a = seed(
            &store,
            "a",
            CanvasFormat::Mermaid,
            "flowchart TD\n  A --> B",
        )
        .await;
        let b = seed(&store, "b", CanvasFormat::Mermaid, "pie\n  \"x\" : 1").await;

        let out = handle(&store, Request::default(), &WorkerConfig::default())
            .await
            .expect("lists");
        assert_eq!(out.count, 2);
        assert_eq!(out.canvases.len(), 2);
        // Same-second creates tie on updated_at; id breaks the tie, so both
        // orders are legal — assert membership and full-record content.
        assert!(out.canvases.contains(&a));
        assert!(out.canvases.contains(&b));
        assert!(out.canvases[0].updated_at >= out.canvases[1].updated_at);
    }

    #[tokio::test]
    async fn the_format_filter_narrows_and_the_cap_truncates() {
        let store = Store::in_memory();
        seed(
            &store,
            "m1",
            CanvasFormat::Mermaid,
            "flowchart TD\n  A --> B",
        )
        .await;
        seed(&store, "m2", CanvasFormat::Mermaid, "mindmap\n  root((x))").await;
        seed(&store, "w", CanvasFormat::Freeform, "{\"elements\": []}").await;

        let boards = handle(
            &store,
            Request {
                format: Some(CanvasFormat::Freeform),
            },
            &WorkerConfig::default(),
        )
        .await
        .expect("lists");
        assert_eq!(boards.count, 1);
        assert_eq!(boards.canvases[0].name, "w");

        let capped = handle(
            &store,
            Request::default(),
            &WorkerConfig {
                max_list: 2,
                ..WorkerConfig::default()
            },
        )
        .await
        .expect("lists");
        assert_eq!(capped.count, 2);
    }

    #[tokio::test]
    async fn an_empty_store_lists_nothing() {
        let store = Store::in_memory();
        let out = handle(&store, Request::default(), &WorkerConfig::default())
            .await
            .expect("lists");
        assert_eq!(out.count, 0);
        assert!(out.canvases.is_empty());
    }
}
