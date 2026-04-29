use std::path::Path;

use iii_sdk::{TriggerRequest, III};
use serde_json::json;

use crate::types::{GateOutcome, GateRunResult, GateSpec};

pub async fn run_gate(iii: &III, gate: &GateSpec, cwd: &Path) -> GateOutcome {
    let cwd_str = cwd.to_string_lossy().into_owned();
    let result = iii
        .trigger(TriggerRequest {
            function_id: gate.function_id.clone(),
            payload: json!({ "cwd": cwd_str }),
            action: None,
            timeout_ms: Some(600_000),
        })
        .await;
    match result {
        Ok(val) => match serde_json::from_value::<GateOutcome>(val.clone()) {
            Ok(o) => o,
            Err(_) => GateOutcome {
                ok: val
                    .get("ok")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                reason: val
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .map(String::from),
            },
        },
        Err(e) => GateOutcome {
            ok: false,
            reason: Some(e.to_string()),
        },
    }
}

pub async fn run_all_gates(iii: &III, gates: &[GateSpec], cwd: &Path) -> Vec<GateRunResult> {
    let mut out = Vec::with_capacity(gates.len());
    for gate in gates {
        let outcome = run_gate(iii, gate, cwd).await;
        out.push(GateRunResult {
            function_id: gate.function_id.clone(),
            description: gate.description.clone(),
            ok: outcome.ok,
            reason: outcome.reason,
        });
    }
    out
}

pub fn all_passed(results: &[GateRunResult]) -> bool {
    results.iter().all(|r| r.ok)
}
