//! `memory::doctor` and `memory::reload` — honesty and recovery.
//!
//! Doctor exists because the most damaging memory failures are silent:
//! stores that report healthy while nothing persists, retrieval that
//! quietly degrades. Every check here exercises the real path end-to-end.

use iii_sdk::protocol::CallRequest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::extract::{CURSOR_SCOPE, ModelReason, PassHealth};
use crate::hooks::EXTRACTION_QUEUE;
use crate::store::Store;
use crate::types::{Confidence, Memory, fingerprint, now_ms};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct DoctorRequest {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DoctorCheck {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DoctorResponse {
    /// True only when every REQUIRED check passed (the store roundtrip; the
    /// rest are reported but advisory).
    pub ok: bool,
    pub checks: Vec<DoctorCheck>,
    /// Extraction passes since boot: how they went, what the model calls
    /// cost and what they wrote.
    pub extraction: PassHealth,
}

pub async fn doctor(deps: &Deps, _req: DoctorRequest) -> Result<DoctorResponse, MemoryError> {
    let cfg = deps.config().await;
    let mut checks = Vec::new();

    // 1. REQUIRED: full save → recall → delete roundtrip in a scratch bank.
    let scratch = format!("doctor-{}", now_ms() % 1_000_000);
    let roundtrip = roundtrip_check(&*deps.store().await, &scratch).await;
    let roundtrip_ok = roundtrip.ok;
    checks.push(roundtrip);

    // 2. Advisory: llm-router reachable (extraction needs it).
    checks.push(
        sibling_check(
            deps,
            "router-reachable",
            "router::models::list",
            json!({}),
            "extraction degrades to manual saves without llm-router",
        )
        .await,
    );

    // 3. Advisory: session-manager reachable (bank selection + transcripts).
    checks.push(
        sibling_check(
            deps,
            "session-manager-reachable",
            "session::list",
            json!({}),
            "hook injection and extraction need session-manager",
        )
        .await,
    );

    // 4. Advisory: the state worker holds the extraction cursor.
    checks.push(
        sibling_check(
            deps,
            "state-reachable",
            "state::get",
            json!({ "scope": CURSOR_SCOPE, "key": "doctor-probe" }),
            "the extraction cursor is lost, so every pass re-reads the newest messages",
        )
        .await,
    );

    // 5. Advisory: an embed provider answers (semantic recall needs it).
    checks.push(embeddings_check(deps, cfg.embeddings_enabled).await);

    // 6. Advisory: extraction passes succeed (a failing one only logs).
    let health = crate::extract::pass_health();
    checks.push(extraction_check(cfg.extraction_enabled, &health));

    // 7. Advisory: the model extraction calls, and why that one.
    let model = if cfg.extraction_enabled {
        Some(crate::extract::resolve_model(&deps.iii, &cfg.extraction_model, 3_000).await)
    } else {
        None
    };
    checks.push(model_check(model));

    // 8. Advisory: jobs the queue gave up on after its retries.
    let stats = deps
        .iii
        .call(CallRequest {
            function_id: "engine::queue::topic_stats".into(),
            payload: json!({ "topic": EXTRACTION_QUEUE }),
            action: None,
            timeout_ms: Some(3_000),
        })
        .await
        .ok();
    checks.push(queue_check(stats.as_ref()));

    Ok(DoctorResponse {
        ok: roundtrip_ok,
        checks,
        extraction: health,
    })
}

async fn embeddings_check(deps: &Deps, enabled: bool) -> DoctorCheck {
    let (ok, detail) = if !enabled {
        (
            true,
            "embeddings disabled by config; recall is lexical-only".into(),
        )
    } else {
        match crate::embed_client::query_vector(deps, "doctor probe").await {
            Some(q) => (true, format!("router::embed answered (model {})", q.model)),
            None => (
                false,
                "router::embed returned no vector; recall runs lexical-only (BM25 + entities)"
                    .into(),
            ),
        }
    };
    DoctorCheck {
        name: "embeddings-available".into(),
        ok,
        detail,
    }
}

fn extraction_check(enabled: bool, health: &PassHealth) -> DoctorCheck {
    let (ok, detail) = match &health.last_error {
        _ if !enabled => (true, "extraction disabled by config".to_string()),
        Some(e) => (
            false,
            format!(
                "the last {} of {} passes since boot failed; nothing is captured until one \
                 succeeds. Last error: {e}",
                health.failed_in_a_row, health.passes
            ),
        ),
        None if health.passes == 0 => (true, "no extraction pass since boot".into()),
        None => {
            let mut detail = format!("{} passes since boot, the latest succeeded", health.passes);
            if health.pending > 0 {
                detail += &format!("; {} left a backlog for the queue to drain", health.pending);
            }
            (true, detail)
        }
    };
    DoctorCheck {
        name: "extraction-passes".into(),
        ok,
        detail,
    }
}

/// `None` when extraction is off: no model is called, so none is resolved.
fn model_check(resolved: Option<Result<(String, ModelReason), String>>) -> DoctorCheck {
    let (ok, detail) = match resolved {
        None => (true, "extraction disabled by config".to_string()),
        Some(Ok((model, ModelReason::Pinned))) => (true, format!("pinned: {model}")),
        Some(Ok((model, reason))) => (
            true,
            format!("automatic: {model} ({})", reason.as_str().replace('_', " ")),
        ),
        Some(Err(e)) => (false, e),
    };
    DoctorCheck {
        name: "extraction-model".into(),
        ok,
        detail,
    }
}

/// `None` when the queue's stats could not be read: advisory, never a
/// failure.
fn queue_check(stats: Option<&Value>) -> DoctorCheck {
    let (ok, detail) = match stats.map(|s| s["dlq_depth"].as_u64()) {
        None | Some(None) => (true, "queue stats unavailable".to_string()),
        Some(Some(0)) => (true, "no dead-lettered jobs reported".to_string()),
        Some(Some(n)) => {
            let (jobs, wait) = if n == 1 {
                ("job", "waits")
            } else {
                ("jobs", "wait")
            };
            (
                false,
                format!(
                    "{n} extraction {jobs} failed every retry and {wait} in the dead-letter \
                     queue. List the queue with engine::queue::dlq_messages, then retry with \
                     iii::queue::redrive or drop one entry with iii::queue::discard_message \
                     (topic {EXTRACTION_QUEUE}). A dead-lettered entry holds the job only. \
                     Each failure is in the `extraction pass failed` WARN, with the model \
                     when the pass reached the model call; the extraction-passes check \
                     keeps only the latest error."
                ),
            )
        }
    };
    DoctorCheck {
        name: "extraction-queue".into(),
        ok,
        detail,
    }
}

fn stated(text: &str) -> Memory {
    let now = now_ms();
    Memory {
        tags: vec![],
        id: fingerprint(text),
        text: text.into(),
        entities: vec![],
        confidence: Confidence::Stated,
        corroboration: 0,
        pinned: false,
        source: None,
        created_at: now,
        updated_at: now,
        invalid_at: None,
        superseded_by: None,
        revision: 0,
        stated_at: Some(now),
    }
}

async fn roundtrip_check(store: &Store, scratch: &str) -> DoctorCheck {
    let result: Result<String, MemoryError> = async {
        let (bank, created) = store.ensure_bank(scratch, Some("doctor scratch")).await?;
        if !created {
            // Not a scratch bank: never write to it, let alone delete it.
            return Err(MemoryError::Storage(format!(
                "scratch bank {scratch} already exists"
            )));
        }
        let saved = bank
            .commit(stated("doctor probe memory: memory roundtrip"))
            .await;
        let hits = match saved {
            Ok(_) => {
                bank.recall("memory roundtrip probe", None, "", 3, 30, false)
                    .await
            }
            Err(_) => Vec::new(),
        };
        // Deleted outright, a failed save included: a scratch bank in
        // .trash is litter nobody will recover.
        store.destroy_bank(scratch).await?;
        saved?;
        if hits.is_empty() {
            return Err(MemoryError::Storage(
                "probe memory was saved but not recallable".into(),
            ));
        }
        Ok("save→recall→delete ok".into())
    }
    .await;
    match result {
        Ok(detail) => DoctorCheck {
            name: "store-roundtrip".into(),
            ok: true,
            detail,
        },
        Err(e) => DoctorCheck {
            name: "store-roundtrip".into(),
            ok: false,
            detail: e.to_string(),
        },
    }
}

async fn sibling_check(
    deps: &Deps,
    name: &str,
    function_id: &str,
    payload: Value,
    why: &str,
) -> DoctorCheck {
    let res = deps
        .iii
        .call(CallRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(3_000),
        })
        .await;
    match res {
        Ok(_) => DoctorCheck {
            name: name.into(),
            ok: true,
            detail: format!("{function_id} reachable"),
        },
        Err(e) => DoctorCheck {
            name: name.into(),
            ok: false,
            detail: format!("{function_id} unreachable ({e}); {why}"),
        },
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ReloadRequest {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ReloadResponse {
    pub ok: bool,
    pub banks: usize,
}

pub async fn reload(deps: &Deps, _req: ReloadRequest) -> Result<ReloadResponse, MemoryError> {
    let store = deps.store().await;
    let banks = store.reload().await?;
    Ok(ReloadResponse { ok: true, banks })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_roundtrip_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let check = roundtrip_check(&store, "doctor-1").await;
        assert!(check.ok, "{}", check.detail);
        assert!(store.bank_names().await.is_empty());
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(left.is_empty(), "no scratch bank, no .trash: {left:?}");
    }

    #[tokio::test]
    async fn the_roundtrip_never_touches_a_bank_it_did_not_create() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("doctor-42", None).await.unwrap();
        let kept = stated("the user keeps release notes in docs/releases");
        bank.commit(kept.clone()).await.unwrap();

        let check = roundtrip_check(&store, "doctor-42").await;
        assert!(!check.ok, "{}", check.detail);
        assert_eq!(store.bank_names().await, ["doctor-42"]);
        // Read back from disk, not the bank's in-memory copy.
        let reopened = Store::open(dir.path().to_path_buf()).unwrap();
        let bank = reopened.bank("doctor-42").await.unwrap();
        assert_eq!(bank.get(&kept.id).await.map(|m| m.text), Some(kept.text));
    }

    #[test]
    fn failing_extraction_passes_fail_the_check() {
        let failing = PassHealth {
            passes: 4,
            failed_in_a_row: 3,
            last_error: Some("router::complete: no provider".into()),
            ..PassHealth::default()
        };
        let check = extraction_check(true, &failing);
        assert!(!check.ok);
        assert!(check.detail.contains("last 3 of 4"), "{}", check.detail);
        assert!(check.detail.contains("no provider"), "{}", check.detail);

        assert!(extraction_check(false, &failing).ok, "disabled by config");
        assert!(
            extraction_check(true, &PassHealth::default()).ok,
            "no pass yet"
        );
        let recovered = PassHealth {
            passes: 4,
            ..PassHealth::default()
        };
        assert!(extraction_check(true, &recovered).ok);

        let draining = PassHealth {
            passes: 5,
            pending: 2,
            ..PassHealth::default()
        };
        let check = extraction_check(true, &draining);
        assert!(check.ok, "a backlog is not a failure");
        assert!(
            check.detail.contains("2 left a backlog"),
            "{}",
            check.detail
        );
    }

    #[test]
    fn dead_lettered_jobs_fail_the_queue_check() {
        let check = queue_check(Some(&json!({ "depth": 0, "dlq_depth": 2 })));
        assert!(!check.ok);
        assert_eq!(check.name, "extraction-queue");
        assert!(
            check
                .detail
                .starts_with("2 extraction jobs failed every retry and wait"),
            "{}",
            check.detail
        );
        for function in [
            "engine::queue::dlq_messages",
            "iii::queue::redrive",
            "iii::queue::discard_message",
            "memory-extraction",
            "extraction-passes",
        ] {
            assert!(check.detail.contains(function), "{}", check.detail);
        }
        let one = queue_check(Some(&json!({ "dlq_depth": 1 }))).detail;
        assert!(
            one.starts_with("1 extraction job failed every retry and waits"),
            "{one}"
        );
        let empty = queue_check(Some(&json!({ "depth": 3, "dlq_depth": 0 })));
        assert!(empty.ok);
        assert_eq!(empty.detail, "no dead-lettered jobs reported");
    }

    #[test]
    fn unreadable_queue_stats_do_not_fail_doctor() {
        for stats in [None, Some(json!({})), Some(json!({ "dlq_depth": "two" }))] {
            let check = queue_check(stats.as_ref());
            assert!(check.ok, "{stats:?}");
            assert_eq!(check.detail, "queue stats unavailable");
        }
    }

    #[test]
    fn a_disabled_extraction_passes_the_model_check() {
        let check = model_check(None);
        assert!(check.ok);
        assert_eq!(check.name, "extraction-model");
        assert_eq!(check.detail, "extraction disabled by config");

        let pinned = model_check(Some(Ok(("a::m".into(), ModelReason::Pinned))));
        assert_eq!((pinned.ok, pinned.detail.as_str()), (true, "pinned: a::m"));
        let automatic = model_check(Some(Ok(("a::m".into(), ModelReason::ProviderDefault))));
        assert_eq!(automatic.detail, "automatic: a::m (provider default)");
        let listed = model_check(Some(Ok(("a::m".into(), ModelReason::FirstListedModel))));
        assert_eq!(listed.detail, "automatic: a::m (first listed model)");
        let none = model_check(Some(Err("no usable model: x; set extraction_model".into())));
        assert!(!none.ok);
        assert!(
            none.detail.ends_with("set extraction_model"),
            "{}",
            none.detail
        );
    }
}
