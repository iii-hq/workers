//! `canvas::create` — make a new canvas from source.
//!
//! The worker mints the stable 8-character id, derives the mermaid family
//! from the source (null for freeform), stamps the timestamps, persists the
//! record over the state bus, and returns the full record.
//!
//! Only cheap gates run here — empty source, the size cap, and (for freeform)
//! that the scene is a JSON object. A mermaid source with an unrecognized
//! header is still stored, with `family: null`; `canvas::validate` is the
//! pre-flight that catches that before an agent commits.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use schemars::JsonSchema;
use serde::Deserialize;

use crate::config::WorkerConfig;
use crate::functions::{family, validate};
use crate::store::{CanvasFormat, CanvasRecord, Store};

pub const ID: &str = "canvas::create";
pub const DESC: &str = "Create a new canvas from mermaid text or an excalidraw scene JSON. \
                        Returns the stored record, including the minted stable id and, for \
                        mermaid, the diagram family derived from the source.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// Human-readable canvas name. Omit for a name derived from the detected
    /// diagram family (`Untitled flowchart`, `Untitled whiteboard`, …).
    #[serde(default)]
    pub name: Option<String>,

    /// Diagram format: `mermaid` or `freeform`. Defaults to `mermaid`.
    #[serde(default)]
    pub format: Option<CanvasFormat>,

    /// The editable source: mermaid text for `mermaid`, an excalidraw scene
    /// JSON string for `freeform`.
    pub source: String,
}

/// The full stored record, as every other `canvas::*` call returns it.
pub type Response = CanvasRecord;

pub async fn handle(store: &Store, req: Request, cfg: &WorkerConfig) -> Result<Response, String> {
    let format = req.format.unwrap_or(CanvasFormat::Mermaid);
    check_source(&req.source, format, cfg)?;

    let family = match format {
        CanvasFormat::Mermaid => family::detect(&req.source),
        CanvasFormat::Freeform => None,
    };
    let name = normalized_name(req.name, format, family.as_deref());
    let now = unix_now();
    let id = mint_id(store).await?;

    let record = CanvasRecord {
        id,
        name,
        format,
        source: req.source,
        family,
        created_at: now,
        updated_at: now,
    };
    store.save(&record).await?;
    Ok(record)
}

/// The gates create and update share: non-empty, under the configured cap,
/// and (freeform) a well-formed scene object.
pub(crate) fn check_source(
    source: &str,
    format: CanvasFormat,
    cfg: &WorkerConfig,
) -> Result<(), String> {
    if source.trim().is_empty() {
        return Err(match format {
            CanvasFormat::Mermaid => "source is empty — pass mermaid text starting with a \
                                      diagram family keyword (canvas::syntax lists them)"
                .to_string(),
            CanvasFormat::Freeform => {
                "source is empty — pass an excalidraw scene JSON object".to_string()
            }
        });
    }
    if source.len() > cfg.max_source_bytes {
        return Err(format!(
            "source is {} bytes; the configured cap is {} bytes — split the diagram or raise \
             max_source_bytes in the worker configuration",
            source.len(),
            cfg.max_source_bytes
        ));
    }
    if format == CanvasFormat::Freeform {
        let issues = validate::check_freeform(source, cfg.max_source_bytes);
        if !issues.is_empty() {
            let messages: Vec<String> = issues.into_iter().map(|i| i.message).collect();
            return Err(format!(
                "freeform source is not a usable excalidraw scene: {}",
                messages.join("; ")
            ));
        }
    }
    Ok(())
}

/// A provided name wins (trimmed); a missing or blank one falls back to a
/// family-derived default so a list of quick sketches stays tellable-apart.
fn normalized_name(name: Option<String>, format: CanvasFormat, family: Option<&str>) -> String {
    if let Some(name) = name {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    match (format, family) {
        (CanvasFormat::Freeform, _) => "Untitled whiteboard".to_string(),
        (CanvasFormat::Mermaid, Some(family)) => format!("Untitled {family}"),
        (CanvasFormat::Mermaid, None) => "Untitled diagram".to_string(),
    }
}

pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

const ID_LEN: usize = 8;
const ID_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

/// One draw from an OS-entropy-seeded hasher (`RandomState` seeds itself from
/// the OS), folded with the clock, the pid and a process counter so two draws
/// in the same nanosecond still differ.
fn random_u64() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    hasher.write_u32(std::process::id());
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    hasher.finish()
}

/// An 8-character lowercase `[a-z0-9]` slug — 36^8 (~2.8e12) possibilities.
pub(crate) fn random_id() -> String {
    let mut bits = random_u64();
    (0..ID_LEN)
        .map(|_| {
            let ch = ID_ALPHABET[(bits % ID_ALPHABET.len() as u64) as usize];
            bits /= ID_ALPHABET.len() as u64;
            ch as char
        })
        .collect()
}

/// Mint an id no stored record already uses. A collision is a ~1-in-10^12
/// event per existing record, but the check is one state read — cheap
/// insurance against handing two canvases one id.
async fn mint_id(store: &Store) -> Result<String, String> {
    for _ in 0..5 {
        let id = random_id();
        if store.load(&id).await?.is_none() {
            return Ok(id);
        }
    }
    Err("could not mint an unused canvas id after 5 attempts".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> WorkerConfig {
        WorkerConfig::default()
    }

    #[tokio::test]
    async fn create_mints_an_8_char_lowercase_id_and_detects_the_family() {
        let store = Store::in_memory();
        let record = handle(
            &store,
            Request {
                name: Some("checkout".into()),
                format: None,
                source: "flowchart TD\n  A --> B\n".into(),
            },
            &cfg(),
        )
        .await
        .expect("creates");

        assert_eq!(record.id.len(), 8);
        assert!(record
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
        assert_eq!(record.name, "checkout");
        assert_eq!(record.format, CanvasFormat::Mermaid);
        assert_eq!(record.family.as_deref(), Some("flowchart"));
        assert_eq!(record.created_at, record.updated_at);
        assert!(record.created_at > 0);

        let stored = store.load(&record.id).await.expect("load").expect("stored");
        assert_eq!(stored, record);
    }

    #[tokio::test]
    async fn format_defaults_to_mermaid_and_name_defaults_from_the_family() {
        let store = Store::in_memory();
        let record = handle(
            &store,
            Request {
                name: None,
                format: None,
                source: "sequenceDiagram\n  A->>B: hi\n".into(),
            },
            &cfg(),
        )
        .await
        .expect("creates");
        assert_eq!(record.format, CanvasFormat::Mermaid);
        assert_eq!(record.name, "Untitled sequenceDiagram");

        let board = handle(
            &store,
            Request {
                name: Some("   ".into()),
                format: Some(CanvasFormat::Freeform),
                source: "{\"elements\": []}".into(),
            },
            &cfg(),
        )
        .await
        .expect("creates");
        assert_eq!(board.name, "Untitled whiteboard");
        assert_eq!(board.family, None);
    }

    #[tokio::test]
    async fn an_unrecognized_mermaid_header_stores_with_null_family() {
        let store = Store::in_memory();
        let record = handle(
            &store,
            Request {
                name: None,
                format: None,
                source: "somethingelse\n  A --> B\n".into(),
            },
            &cfg(),
        )
        .await
        .expect("creates");
        assert_eq!(record.family, None);
        assert_eq!(record.name, "Untitled diagram");
    }

    #[tokio::test]
    async fn empty_oversized_and_malformed_sources_are_rejected() {
        let store = Store::in_memory();

        let empty = handle(
            &store,
            Request {
                name: None,
                format: None,
                source: "  \n".into(),
            },
            &cfg(),
        )
        .await
        .expect_err("empty rejected");
        assert!(empty.contains("empty"));

        let small_cap = WorkerConfig {
            max_source_bytes: 10,
            ..WorkerConfig::default()
        };
        let big = handle(
            &store,
            Request {
                name: None,
                format: None,
                source: "flowchart TD\n  A --> B\n".into(),
            },
            &small_cap,
        )
        .await
        .expect_err("oversize rejected");
        assert!(big.contains("cap"), "{big}");

        let bad_scene = handle(
            &store,
            Request {
                name: None,
                format: Some(CanvasFormat::Freeform),
                source: "not json".into(),
            },
            &cfg(),
        )
        .await
        .expect_err("bad scene rejected");
        assert!(bad_scene.contains("excalidraw"), "{bad_scene}");
    }

    #[test]
    fn random_ids_have_the_right_shape_and_do_not_repeat_cheaply() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let id = random_id();
            assert_eq!(id.len(), 8);
            assert!(id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
            seen.insert(id);
        }
        assert!(
            seen.len() > 990,
            "1000 draws produced only {} distinct ids",
            seen.len()
        );
    }
}
