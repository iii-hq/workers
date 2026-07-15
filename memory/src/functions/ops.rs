//! `memory::doctor` and `memory::reload` — honesty and recovery.
//!
//! Doctor exists because the most damaging memory failures are silent:
//! stores that report healthy while nothing persists, retrieval that
//! quietly degrades. Every check here exercises the real path end-to-end.

use iii_sdk::protocol::TriggerRequest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::types::{fingerprint, now_ms, Confidence, Fact};

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
    /// True only when every REQUIRED check passed (sibling reachability is
    /// reported but advisory).
    pub ok: bool,
    pub checks: Vec<DoctorCheck>,
}

pub async fn doctor(deps: &Deps, _req: DoctorRequest) -> Result<DoctorResponse, MemoryError> {
    let mut checks = Vec::new();

    // 1. REQUIRED: full save → recall → trash roundtrip in a scratch bank.
    let roundtrip = roundtrip_check(deps).await;
    let roundtrip_ok = roundtrip.ok;
    checks.push(roundtrip);

    // 2. Advisory: llm-router reachable (extraction needs it).
    checks.push(
        sibling_check(
            deps,
            "router-reachable",
            "router::models::list",
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
            "hook injection and extraction need session-manager",
        )
        .await,
    );

    Ok(DoctorResponse {
        ok: roundtrip_ok,
        checks,
    })
}

async fn roundtrip_check(deps: &Deps) -> DoctorCheck {
    let store = deps.store().await;
    let scratch = format!("doctor-{}", now_ms() % 1_000_000);
    let probe_text = "doctor probe fact: memory roundtrip";
    let result: Result<String, MemoryError> = async {
        let (bank, _) = store.ensure_bank(&scratch, Some("doctor scratch")).await?;
        let now = now_ms();
        bank.commit(Fact {
            id: fingerprint(probe_text),
            text: probe_text.into(),
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
        })
        .await?;
        let hits = bank
            .recall("memory roundtrip probe", None, 3, 30, false)
            .await;
        let trashed = store.trash_bank(&scratch).await?;
        if hits.is_empty() {
            return Err(MemoryError::Storage(
                "probe fact was saved but not recallable".into(),
            ));
        }
        Ok(format!("save→recall→trash ok (scratch at {trashed})"))
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

async fn sibling_check(deps: &Deps, name: &str, function_id: &str, why: &str) -> DoctorCheck {
    let res = deps
        .iii
        .trigger(TriggerRequest {
            function_id: function_id.into(),
            payload: json!({}),
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
