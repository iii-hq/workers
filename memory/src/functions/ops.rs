//! `memory::doctor` and `memory::reload` — honesty and recovery.
//!
//! Doctor exists because the most damaging memory failures are silent:
//! stores that report healthy while nothing persists, retrieval that
//! quietly degrades. Every check here exercises the real path end-to-end.

use iii_sdk::protocol::TriggerRequest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::extract::{PassHealth, CURSOR_SCOPE};
use crate::store::Store;
use crate::types::{fingerprint, now_ms, Confidence, Memory};

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
    checks.push(extraction_check(
        cfg.extraction_enabled,
        &crate::extract::pass_health(),
    ));

    Ok(DoctorResponse {
        ok: roundtrip_ok,
        checks,
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
        None => (
            true,
            format!("{} passes since boot, the latest succeeded", health.passes),
        ),
    };
    DoctorCheck {
        name: "extraction-passes".into(),
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
        .trigger(TriggerRequest {
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
    }
}
