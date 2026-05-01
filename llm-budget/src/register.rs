//! Register `budget::*` functions on the iii bus. Direct port of
//! roster/workers/llm-budget/src/worker.ts.

use chrono::Utc;
use iii_sdk::{FunctionRef, IIIError, RegisterFunctionMessage, TriggerRequest, Value, III};
use serde_json::json;

use crate::ops::{maybe_roll_over, prune_exemptions, with_budget_lock};
use crate::periods::{days_elapsed, days_remaining, next_period_start, period_start, Period};
use crate::store::{
    delete_budget, list_all, list_spend_logs, load_budget, save_budget, save_reset_log,
    save_spend_log, Alert, Budget, Exemption, SpendLogEntry,
};

const FN_PREFIX: &str = "budget";
const EXEMPT_TTL_MS: i64 = 24 * 60 * 60 * 1000;

pub struct BudgetFunctionRefs {
    pub create: FunctionRef,
    pub list: FunctionRef,
    pub get: FunctionRef,
    pub update: FunctionRef,
    pub delete: FunctionRef,
    pub check: FunctionRef,
    pub record: FunctionRef,
    pub reset: FunctionRef,
    pub alert_set: FunctionRef,
    pub usage: FunctionRef,
    pub forecast: FunctionRef,
    pub enforce: FunctionRef,
    pub exempt: FunctionRef,
    pub pause: FunctionRef,
}

impl BudgetFunctionRefs {
    pub fn unregister_all(self) {
        for r in [
            self.create,
            self.list,
            self.get,
            self.update,
            self.delete,
            self.check,
            self.record,
            self.reset,
            self.alert_set,
            self.usage,
            self.forecast,
            self.enforce,
            self.exempt,
            self.pause,
        ] {
            r.unregister();
        }
    }
}

pub async fn register_with_iii(iii: &III) -> anyhow::Result<BudgetFunctionRefs> {
    // ── budget::create ────────────────────────────────────────────────────────
    let create = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::create"))
                .with_description("Create a budget with ceiling + period.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let ceiling = payload
                        .get("ceiling_usd")
                        .and_then(Value::as_f64)
                        .ok_or_else(|| IIIError::Handler("ceiling_usd must be > 0".into()))?;
                    if !ceiling.is_finite() || ceiling <= 0.0 {
                        return Err(IIIError::Handler("ceiling_usd must be > 0".into()));
                    }
                    let period = parse_period(&payload, "period")?;
                    let now_ms = Utc::now().timestamp_millis();
                    let start = period_start(period, now_ms);
                    let b = Budget {
                        id: uuid::Uuid::new_v4().to_string(),
                        workspace_id: payload
                            .get("workspace_id")
                            .and_then(Value::as_str)
                            .map(String::from),
                        agent_id: payload
                            .get("agent_id")
                            .and_then(Value::as_str)
                            .map(String::from),
                        name: payload
                            .get("name")
                            .and_then(Value::as_str)
                            .map(String::from),
                        ceiling_usd: ceiling,
                        period,
                        spent_usd: 0.0,
                        period_start_at: start,
                        period_resets_at: next_period_start(period, start),
                        enforced: true,
                        paused: false,
                        alerts: Vec::new(),
                        exemptions: Vec::new(),
                        created_at: now_ms,
                        updated_at: now_ms,
                    };
                    save_budget(&iii, &b).await.map_err(io_err)?;
                    Ok(json!({ "budget_id": b.id }))
                }
            },
        ))
    };

    // ── budget::list ──────────────────────────────────────────────────────────
    let list = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::list"))
                .with_description("List budgets, newest first.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let ws = payload
                        .get("workspace_id")
                        .and_then(Value::as_str)
                        .map(String::from);
                    let mut all = list_all(&iii).await.map_err(io_err)?;
                    if let Some(ws) = ws {
                        all.retain(|b| b.workspace_id.as_deref() == Some(ws.as_str()));
                    }
                    all.sort_by_key(|b| -b.created_at);
                    Ok(json!({ "budgets": all }))
                }
            },
        ))
    };

    // ── budget::get ───────────────────────────────────────────────────────────
    let get = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::get"))
                .with_description("Fetch a budget by id.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let id = required_str(&payload, "budget_id")?;
                    let b = load_budget(&iii, &id)
                        .await
                        .map_err(io_err)?
                        .ok_or_else(|| IIIError::Handler(format!("budget not found: {id}")))?;
                    Ok(json!({ "budget": b }))
                }
            },
        ))
    };

    // ── budget::update ────────────────────────────────────────────────────────
    // Port of roster/workers/llm-budget/src/worker.ts lines 180-237.
    // Whitelists: name, ceiling_usd, period, enforced, paused.
    // Period change triggers archive of the just-closed window + re-anchor.
    let update = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::update"))
                .with_description("Update a whitelisted set of budget fields.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let id = required_str(&payload, "budget_id")?;
                    let patch = match payload.get("patch") {
                        Some(Value::Object(m)) => m.clone(),
                        _ => return Err(IIIError::Handler("patch must be an object".into())),
                    };
                    with_budget_lock(&id, || async {
                        let ts = Utc::now().timestamp_millis();
                        let current = load_budget(&iii, &id)
                            .await?
                            .ok_or_else(|| anyhow::anyhow!("budget not found: {id}"))?;

                        // Whitelist: silently drop any field not in the allowed set.
                        // Validate each allowed field.
                        let new_ceiling = if let Some(v) = patch.get("ceiling_usd") {
                            let c = v.as_f64().ok_or_else(|| {
                                anyhow::anyhow!("ceiling_usd must be a positive finite number")
                            })?;
                            if !c.is_finite() || c <= 0.0 {
                                return Err(anyhow::anyhow!(
                                    "ceiling_usd must be a positive finite number"
                                ));
                            }
                            Some(c)
                        } else {
                            None
                        };

                        let new_period = if let Some(v) = patch.get("period") {
                            let s = v
                                .as_str()
                                .ok_or_else(|| anyhow::anyhow!("period must be a string"))?;
                            Some(parse_period_str(s).map_err(|e| anyhow::anyhow!("{e}"))?)
                        } else {
                            None
                        };

                        let new_enforced = if let Some(v) = patch.get("enforced") {
                            match v {
                                Value::Bool(b) => Some(*b),
                                _ => {
                                    return Err(anyhow::anyhow!(
                                        "enforced must be a boolean (got {v})"
                                    ))
                                }
                            }
                        } else {
                            None
                        };

                        let new_paused = if let Some(v) = patch.get("paused") {
                            match v {
                                Value::Bool(b) => Some(*b),
                                _ => {
                                    return Err(anyhow::anyhow!(
                                        "paused must be a boolean (got {v})"
                                    ))
                                }
                            }
                        } else {
                            None
                        };

                        let new_name = patch.get("name").and_then(Value::as_str).map(String::from);

                        // If period kind is changing, first roll the old period forward so any
                        // elapsed zero-spend boundaries are archived before the switch.
                        let period_changing =
                            new_period.is_some() && new_period != Some(current.period);
                        let rolled = if period_changing {
                            maybe_roll_over(&iii, current.clone(), ts).await?
                        } else {
                            current.clone()
                        };

                        // Build the updated budget by applying whitelisted changes.
                        let mut next = Budget {
                            ceiling_usd: new_ceiling.unwrap_or(rolled.ceiling_usd),
                            period: new_period.unwrap_or(rolled.period),
                            enforced: new_enforced.unwrap_or(rolled.enforced),
                            paused: new_paused.unwrap_or(rolled.paused),
                            name: new_name.or_else(|| rolled.name.clone()),
                            updated_at: ts,
                            ..rolled.clone()
                        };

                        if period_changing {
                            // Archive the just-closed window under the old period.
                            save_spend_log(
                                &iii,
                                &rolled.id,
                                rolled.period_start_at,
                                &SpendLogEntry {
                                    budget_id: rolled.id.clone(),
                                    period_start: rolled.period_start_at,
                                    period_end: ts,
                                    spent_usd: rolled.spent_usd,
                                    records_count: 0,
                                },
                            )
                            .await?;
                            // Re-anchor to the new period.
                            let new_p = next.period;
                            next.period_start_at = period_start(new_p, ts);
                            next.period_resets_at = next_period_start(new_p, next.period_start_at);
                            next.spent_usd = 0.0;
                            for a in &mut next.alerts {
                                a.last_fired_period_start = None;
                            }
                        }

                        save_budget(&iii, &next).await?;
                        Ok(json!({ "budget": next }))
                    })
                    .await
                    .map_err(io_err)
                }
            },
        ))
    };

    // ── budget::delete ────────────────────────────────────────────────────────
    let delete = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::delete"))
                .with_description("Delete a budget.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let id = required_str(&payload, "budget_id")?;
                    with_budget_lock(&id, || async {
                        delete_budget(&iii, &id).await?;
                        Ok(json!({ "ok": true }))
                    })
                    .await
                    .map_err(io_err)
                }
            },
        ))
    };

    // ── budget::check ─────────────────────────────────────────────────────────
    let check = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::check"))
                .with_description("Check whether a budget allows an estimated spend.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let id = required_str(&payload, "budget_id")?;
                    with_budget_lock(&id, || async {
                        let est = payload.get("estimated_cost_usd").and_then(Value::as_f64).unwrap_or(0.0);
                        if !est.is_finite() || est < 0.0 {
                            return Err(anyhow::anyhow!("estimated_cost_usd must be a finite number >= 0"));
                        }
                        let now_ms = Utc::now().timestamp_millis();
                        let loaded = load_budget(&iii, &id).await?
                            .ok_or_else(|| anyhow::anyhow!("budget not found: {id}"))?;
                        let rolled = maybe_roll_over(&iii, loaded.clone(), now_ms).await?;
                        let b = prune_exemptions(rolled.clone(), now_ms);
                        if !budgets_equal_for_persist(&loaded, &b) {
                            save_budget(&iii, &b).await?;
                        }
                        let remaining = b.ceiling_usd - b.spent_usd;
                        if b.paused {
                            return Ok(json!({ "allowed": true, "remaining_usd": remaining, "reason": "paused" }));
                        }
                        if !b.enforced {
                            return Ok(json!({ "allowed": true, "remaining_usd": remaining, "reason": "not_enforced" }));
                        }
                        if let Some(pid) = payload.get("principal_id").and_then(Value::as_str) {
                            if b.exemptions.iter().any(|e| e.principal_id == pid) {
                                return Ok(json!({ "allowed": true, "remaining_usd": remaining, "reason": "exempt" }));
                            }
                        }
                        if remaining < est {
                            return Ok(json!({ "allowed": false, "remaining_usd": remaining, "reason": "ceiling_exceeded" }));
                        }
                        Ok(json!({ "allowed": true, "remaining_usd": remaining }))
                    }).await.map_err(io_err)
                }
            },
        ))
    };

    // ── budget::record ────────────────────────────────────────────────────────
    let record = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::record"))
                .with_description("Record a spend, fire matching alerts.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let id = required_str(&payload, "budget_id")?;
                    let cost = payload.get("cost_usd").and_then(Value::as_f64)
                        .ok_or_else(|| IIIError::Handler("cost_usd must be >= 0".into()))?;
                    if !cost.is_finite() || cost < 0.0 {
                        return Err(IIIError::Handler("cost_usd must be >= 0".into()));
                    }
                    with_budget_lock(&id, || async {
                        let now_ms = Utc::now().timestamp_millis();
                        let loaded = load_budget(&iii, &id).await?
                            .ok_or_else(|| anyhow::anyhow!("budget not found: {id}"))?;
                        let mut b = maybe_roll_over(&iii, loaded, now_ms).await?;
                        b.spent_usd += cost;
                        b.updated_at = now_ms;

                        let ratio = if b.ceiling_usd > 0.0 { b.spent_usd / b.ceiling_usd } else { 0.0 };
                        let mut pending: Vec<Alert> = Vec::new();
                        for a in &mut b.alerts {
                            if ratio >= a.threshold_pct && a.last_fired_period_start != Some(b.period_start_at) {
                                pending.push(a.clone());
                                a.last_fired_period_start = Some(b.period_start_at);
                            }
                        }
                        save_budget(&iii, &b).await?;

                        for a in pending {
                            // Mirror the TS spread `{ ...(callback_payload ?? {}), <system fields> }`:
                            // a non-object callback_payload (null, number, array) coerces to {}
                            // before the system fields overlay, so they are ALWAYS present.
                            let mut map = serde_json::Map::new();
                            if let Some(obj) = a.callback_payload.as_ref().and_then(Value::as_object) {
                                for (k, v) in obj {
                                    map.insert(k.clone(), v.clone());
                                }
                            }
                            map.insert("alert_id".into(), json!(a.alert_id));
                            map.insert("budget_id".into(), json!(b.id));
                            map.insert("spent_usd".into(), json!(b.spent_usd));
                            map.insert("ceiling_usd".into(), json!(b.ceiling_usd));
                            map.insert("threshold_pct".into(), json!(a.threshold_pct));
                            let cb_payload = Value::Object(map);
                            let cb_id = a.callback_function_id.clone();
                            let iii_cb = iii.clone();
                            tokio::spawn(async move {
                                let _ = iii_cb.trigger(TriggerRequest {
                                    function_id: cb_id,
                                    payload: cb_payload,
                                    action: None,
                                    timeout_ms: None,
                                }).await;
                            });
                        }
                        Ok(json!({ "spent_usd": b.spent_usd, "remaining_usd": b.ceiling_usd - b.spent_usd }))
                    }).await.map_err(io_err)
                }
            },
        ))
    };

    // ── budget::reset ─────────────────────────────────────────────────────────
    // Port of roster/workers/llm-budget/src/worker.ts lines 350-398.
    // Roll forward first; archive current window via save_reset_log with a fresh
    // uuid suffix; re-anchor period boundaries + spent_usd = 0; clear alert
    // last_fired. Do NOT rethrow if the archive save fails after budget committed.
    let reset = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::reset"))
                .with_description("Reset spent_usd, archive prior period.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let id = required_str(&payload, "budget_id")?;
                    with_budget_lock(&id, || async {
                        let ts = Utc::now().timestamp_millis();
                        let loaded = load_budget(&iii, &id)
                            .await?
                            .ok_or_else(|| anyhow::anyhow!("budget not found: {id}"))?;
                        // Roll forward first so any skipped zero-spend periods are archived
                        // under their own boundaries before we archive the current window.
                        let b = maybe_roll_over(&iii, loaded, ts).await?;
                        let previous = b.spent_usd;

                        let start = period_start(b.period, ts);
                        let mut reset_alerts = b.alerts.clone();
                        for a in &mut reset_alerts {
                            a.last_fired_period_start = None;
                        }
                        let reset_budget = Budget {
                            spent_usd: 0.0,
                            period_start_at: start,
                            period_resets_at: next_period_start(b.period, start),
                            updated_at: ts,
                            alerts: reset_alerts,
                            ..b.clone()
                        };
                        // Save the budget first. If it fails, no orphan reset log exists
                        // and the caller can retry cleanly.
                        save_budget(&iii, &reset_budget).await?;

                        // Archive under a unique reset key so it doesn't collide with
                        // the post-reset live period boundary.
                        let suffix = uuid::Uuid::new_v4().to_string();
                        if let Err(e) = save_reset_log(
                            &iii,
                            &b.id,
                            b.period_start_at,
                            ts,
                            &suffix,
                            &SpendLogEntry {
                                budget_id: b.id.clone(),
                                period_start: b.period_start_at,
                                period_end: ts,
                                spent_usd: previous,
                                records_count: 0,
                            },
                        )
                        .await
                        {
                            tracing::error!(
                                budget_id = %b.id,
                                previous_spent_usd = previous,
                                error = %e,
                                "reset archive save failed after budget reset committed"
                            );
                            // Don't rethrow — the budget is already reset; rethrowing would
                            // mislead the caller into thinking the reset itself failed.
                        }
                        Ok(json!({ "budget_id": b.id, "previous_spent_usd": previous }))
                    })
                    .await
                    .map_err(io_err)
                }
            },
        ))
    };

    // ── budget::alert_set ─────────────────────────────────────────────────────
    // Port of roster/workers/llm-budget/src/worker.ts lines 400-426.
    // Validates threshold_pct ∈ (0, 1]; appends a new Alert with fresh uuid.
    let alert_set = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::alert_set"))
                .with_description("Add an alert to a budget.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let id = required_str(&payload, "budget_id")?;
                    with_budget_lock(&id, || async {
                        let threshold_pct =
                            match payload.get("threshold_pct").and_then(Value::as_f64) {
                                Some(v) if v.is_finite() => v,
                                Some(_) => {
                                    return Err(anyhow::anyhow!(
                                        "threshold_pct must be a finite number"
                                    ))
                                }
                                None => {
                                    return Err(anyhow::anyhow!(
                                        "threshold_pct must be a finite number"
                                    ))
                                }
                            };
                        if threshold_pct <= 0.0 || threshold_pct > 1.0 {
                            return Err(anyhow::anyhow!("threshold_pct must be in (0, 1]"));
                        }
                        let callback_function_id = payload
                            .get("callback_function_id")
                            .and_then(Value::as_str)
                            .map(String::from)
                            .ok_or_else(|| {
                                anyhow::anyhow!("missing required field: callback_function_id")
                            })?;
                        let callback_payload = payload.get("callback_payload").cloned();

                        let b = load_budget(&iii, &id)
                            .await?
                            .ok_or_else(|| anyhow::anyhow!("budget not found: {id}"))?;
                        let alert = Alert {
                            alert_id: uuid::Uuid::new_v4().to_string(),
                            threshold_pct,
                            callback_function_id,
                            callback_payload,
                            last_fired_period_start: None,
                        };
                        let alert_id = alert.alert_id.clone();
                        let mut alerts = b.alerts.clone();
                        alerts.push(alert);
                        let next = Budget {
                            alerts,
                            updated_at: Utc::now().timestamp_millis(),
                            ..b
                        };
                        save_budget(&iii, &next).await?;
                        Ok(json!({ "alert_id": alert_id }))
                    })
                    .await
                    .map_err(io_err)
                }
            },
        ))
    };

    // ── budget::usage ─────────────────────────────────────────────────────────
    // Port of roster/workers/llm-budget/src/worker.ts lines 428-482.
    // Window must align with the budget period (or be "all").
    // Aggregates archived spend logs + the live period.
    let usage = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::usage"))
                .with_description("Aggregate spend over a window.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let id = required_str(&payload, "budget_id")?;
                    with_budget_lock(&id, || async {
                        let ts = Utc::now().timestamp_millis();
                        let loaded = load_budget(&iii, &id).await?
                            .ok_or_else(|| anyhow::anyhow!("budget not found: {id}"))?;
                        // Roll forward so we don't report the previous period as current.
                        let b = maybe_roll_over(&iii, loaded.clone(), ts).await?;
                        if !budgets_equal_for_persist(&loaded, &b) {
                            save_budget(&iii, &b).await?;
                        }

                        let window = payload.get("window")
                            .and_then(Value::as_str)
                            .unwrap_or("all")
                            .to_string();

                        // Validate window value.
                        match window.as_str() {
                            "all" | "day" | "week" | "month" => {}
                            other => return Err(anyhow::anyhow!("invalid window: {other}")),
                        }

                        // Window must align with the budget period or be "all".
                        let period_str = period_to_str(b.period);
                        if window != "all" && window != period_str {
                            return Err(anyhow::anyhow!(
                                "window '{window}' does not align with budget period '{period_str}'. Use window: '{period_str}' or 'all'."
                            ));
                        }

                        let cutoff = if window == "all" {
                            0i64
                        } else {
                            let window_period = parse_period_str(&window)
                                .map_err(|e| anyhow::anyhow!("{e}"))?;
                            period_start(window_period, ts)
                        };

                        let logs = list_spend_logs(&iii, &id).await?;
                        // Exclude archived entries for the current period_start_at —
                        // the live budget contributes that period below. Prevents
                        // double-counting after reset or rollover.
                        let relevant: Vec<_> = logs.iter()
                            .filter(|l| l.period_start >= cutoff && l.period_start != b.period_start_at)
                            .collect();

                        let mut by_period: Vec<serde_json::Value> = relevant.iter()
                            .map(|l| json!({ "period": l.period_start, "spent": l.spent_usd }))
                            .collect();
                        // Sort ascending by period start.
                        by_period.sort_by_key(|v| v.get("period").and_then(Value::as_i64).unwrap_or(0));

                        if b.period_start_at >= cutoff {
                            by_period.push(json!({ "period": b.period_start_at, "spent": b.spent_usd }));
                        }

                        let spent: f64 = by_period.iter()
                            .filter_map(|v| v.get("spent").and_then(Value::as_f64))
                            .sum();
                        let records_count = by_period.len();

                        Ok(json!({
                            "spent_usd": spent,
                            "by_period": by_period,
                            "records_count": records_count,
                        }))
                    }).await.map_err(io_err)
                }
            },
        ))
    };

    // ── budget::forecast ──────────────────────────────────────────────────────
    let forecast = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::forecast"))
                .with_description("Project spend through period end.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let id = required_str(&payload, "budget_id")?;
                    with_budget_lock(&id, || async {
                        let now_ms = Utc::now().timestamp_millis();
                        let loaded = load_budget(&iii, &id)
                            .await?
                            .ok_or_else(|| anyhow::anyhow!("budget not found: {id}"))?;
                        let b = maybe_roll_over(&iii, loaded.clone(), now_ms).await?;
                        if !budgets_equal_for_persist(&loaded, &b) {
                            save_budget(&iii, &b).await?;
                        }
                        let elapsed = days_elapsed(b.period_start_at, now_ms);
                        let rate = b.spent_usd / elapsed;
                        let projected_month = rate * 30.0;
                        let remaining_budget = b.ceiling_usd - b.spent_usd;
                        let days_until_breach = if rate > 0.0 && remaining_budget > 0.0 {
                            Some(remaining_budget / rate)
                        } else {
                            None
                        };
                        let remaining_days = days_remaining(now_ms, b.period_resets_at);
                        let on_track = rate * remaining_days <= remaining_budget;
                        Ok(json!({
                            "projected_month_usd": projected_month,
                            "on_track": on_track,
                            "days_until_breach": days_until_breach,
                        }))
                    })
                    .await
                    .map_err(io_err)
                }
            },
        ))
    };

    // ── budget::enforce ───────────────────────────────────────────────────────
    let enforce = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::enforce"))
                .with_description("Toggle enforcement on a budget.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let id = required_str(&payload, "budget_id")?;
                    let enforced = payload
                        .get("enforced")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| IIIError::Handler("enforced must be a boolean".into()))?;
                    with_budget_lock(&id, || async {
                        let loaded = load_budget(&iii, &id)
                            .await?
                            .ok_or_else(|| anyhow::anyhow!("budget not found: {id}"))?;
                        let next = Budget {
                            enforced,
                            updated_at: Utc::now().timestamp_millis(),
                            ..loaded
                        };
                        save_budget(&iii, &next).await?;
                        Ok(json!({ "budget_id": next.id, "enforced": next.enforced }))
                    })
                    .await
                    .map_err(io_err)
                }
            },
        ))
    };

    // ── budget::exempt ────────────────────────────────────────────────────────
    let exempt = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::exempt"))
                .with_description("Add a 24h exemption for a principal.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let id = required_str(&payload, "budget_id")?;
                    let principal_id = required_str(&payload, "principal_id")?;
                    let reason = required_str(&payload, "reason")?;
                    with_budget_lock(&id, || async {
                        let now_ms = Utc::now().timestamp_millis();
                        let loaded = load_budget(&iii, &id)
                            .await?
                            .ok_or_else(|| anyhow::anyhow!("budget not found: {id}"))?;
                        let pruned = prune_exemptions(loaded, now_ms);
                        let mut without: Vec<Exemption> = pruned
                            .exemptions
                            .iter()
                            .filter(|e| e.principal_id != principal_id)
                            .cloned()
                            .collect();
                        let exemption = Exemption {
                            principal_id,
                            reason,
                            expires_at: now_ms + EXEMPT_TTL_MS,
                        };
                        without.push(exemption.clone());
                        let next = Budget {
                            exemptions: without,
                            updated_at: now_ms,
                            ..pruned
                        };
                        save_budget(&iii, &next).await?;
                        Ok(json!({ "budget_id": next.id, "expires_at": exemption.expires_at }))
                    })
                    .await
                    .map_err(io_err)
                }
            },
        ))
    };

    // ── budget::pause ─────────────────────────────────────────────────────────
    let pause = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::pause"))
                .with_description("Pause or resume a budget.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let id = required_str(&payload, "budget_id")?;
                    let paused = payload
                        .get("paused")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| IIIError::Handler("paused must be a boolean".into()))?;
                    with_budget_lock(&id, || async {
                        let loaded = load_budget(&iii, &id)
                            .await?
                            .ok_or_else(|| anyhow::anyhow!("budget not found: {id}"))?;
                        let next = Budget {
                            paused,
                            updated_at: Utc::now().timestamp_millis(),
                            ..loaded
                        };
                        save_budget(&iii, &next).await?;
                        Ok(json!({ "budget_id": next.id, "paused": next.paused }))
                    })
                    .await
                    .map_err(io_err)
                }
            },
        ))
    };

    Ok(BudgetFunctionRefs {
        create,
        list,
        get,
        update,
        delete,
        check,
        record,
        reset,
        alert_set,
        usage,
        forecast,
        enforce,
        exempt,
        pause,
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn io_err<E: std::fmt::Display>(e: E) -> IIIError {
    IIIError::Handler(e.to_string())
}

fn parse_period(payload: &Value, field: &str) -> Result<Period, IIIError> {
    match payload.get(field).and_then(Value::as_str) {
        Some("day") => Ok(Period::Day),
        Some("week") => Ok(Period::Week),
        Some("month") => Ok(Period::Month),
        Some(other) => Err(IIIError::Handler(format!(
            "invalid period: {other} (expected 'day' | 'week' | 'month')"
        ))),
        None => Err(IIIError::Handler(format!(
            "missing required field: {field}"
        ))),
    }
}

fn parse_period_str(s: &str) -> Result<Period, String> {
    match s {
        "day" => Ok(Period::Day),
        "week" => Ok(Period::Week),
        "month" => Ok(Period::Month),
        other => Err(format!(
            "invalid period: {other} (expected 'day' | 'week' | 'month')"
        )),
    }
}

fn period_to_str(p: Period) -> &'static str {
    match p {
        Period::Day => "day",
        Period::Week => "week",
        Period::Month => "month",
    }
}

fn required_str(payload: &Value, field: &str) -> Result<String, IIIError> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| IIIError::Handler(format!("missing required field: {field}")))
}

fn budgets_equal_for_persist(a: &Budget, b: &Budget) -> bool {
    serde_json::to_value(a).unwrap_or(Value::Null) == serde_json::to_value(b).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_period_ok() {
        assert!(matches!(
            parse_period(&json!({ "period": "day" }), "period").unwrap(),
            Period::Day
        ));
    }

    #[test]
    fn parse_period_rejects_unknown() {
        assert!(parse_period(&json!({ "period": "year" }), "period").is_err());
    }
}
