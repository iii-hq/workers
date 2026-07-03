//! Per-session settings: `approval_settings/<session_id>`, created
//! lazily on first mutation only (approval-gate.md § State lifecycle).
//! Reads never write — the *effective* settings are the stored record
//! when one exists, else the configuration defaults computed in memory.

use iii_sdk::IIIClient;
use serde_json::Value;

use crate::config::WorkerConfig;
use crate::error::ApprovalError;
use crate::state;
use crate::types::{
    now_ms, validate_id, AlwaysAllowEntry, ApprovalSettings, GrantedBy, SettingsSource,
};

pub const SETTINGS_SCOPE: &str = "approval_settings";

/// Tolerant parse: missing fields fall back to their defaults (serde
/// `#[serde(default)]` on every field); a non-object or unparseable value
/// is treated as no record.
pub fn parse_settings(value: &Value) -> Option<ApprovalSettings> {
    if !value.is_object() {
        return None;
    }
    match serde_json::from_value::<ApprovalSettings>(value.clone()) {
        Ok(settings) => Some(settings),
        Err(e) => {
            tracing::warn!(error = %e, "unparseable approval_settings record; treating as absent");
            None
        }
    }
}

/// The in-memory settings a session with no stored record runs on:
/// configuration defaults, seed entries marked `granted_by: "seed"`.
pub fn seeded_from(cfg: &WorkerConfig, granted_at: i64) -> ApprovalSettings {
    ApprovalSettings {
        mode: cfg.default_mode,
        always_allow: cfg
            .auto_allow_seed()
            .iter()
            .map(|function_id| AlwaysAllowEntry {
                function_id: function_id.clone(),
                granted_at,
                granted_by: GrantedBy::Seed,
            })
            .collect(),
        approved_always: Vec::new(),
        mode_set_at: 0,
    }
}

pub fn effective(
    stored: Option<ApprovalSettings>,
    cfg: &WorkerConfig,
) -> (ApprovalSettings, SettingsSource) {
    match stored {
        Some(settings) => (settings, SettingsSource::Stored),
        None => (seeded_from(cfg, 0), SettingsSource::Defaults),
    }
}

/// Hot-path read for the gate: any failure (state outage, absent record,
/// garbage) degrades to `None` → configuration defaults. Safe because the
/// default mode never widens beyond what the deployment configured.
pub async fn read_tolerant(iii: &IIIClient, session_id: &str) -> Option<ApprovalSettings> {
    let reply = state::get(iii, SETTINGS_SCOPE, session_id).await;
    match reply {
        Ok(value) => parse_settings(&value),
        Err(e) => {
            tracing::warn!(session_id, error = %e, "settings read failed; using defaults");
            None
        }
    }
}

/// Strict read for mutations: a state outage is an error — re-seeding
/// over an unreadable record would clobber it.
pub async fn read_strict(
    iii: &IIIClient,
    session_id: &str,
) -> Result<Option<ApprovalSettings>, ApprovalError> {
    let reply = state::get(iii, SETTINGS_SCOPE, session_id)
        .await
        .map_err(|e| ApprovalError::StateUnavailable(format!("settings read failed: {e}")))?;
    Ok(parse_settings(&reply))
}

/// First mutation materializes the record from the current defaults, then
/// applies the mutation; from then on the stored record wins (a later
/// seed change does not retroactively edit it). The whole record is
/// written in one `state::set` — mutations are human-driven and rare, so
/// read-modify-write of one small record is sufficient.
pub async fn materialize_and<F>(
    iii: &IIIClient,
    session_id: &str,
    cfg: &WorkerConfig,
    mutate: F,
) -> Result<ApprovalSettings, ApprovalError>
where
    F: FnOnce(ApprovalSettings, i64) -> ApprovalSettings,
{
    validate_id("session_id", session_id)?;
    let now = now_ms();
    let base = read_strict(iii, session_id)
        .await?
        .unwrap_or_else(|| seeded_from(cfg, now));
    let next = mutate(base, now);
    state::set(
        iii,
        SETTINGS_SCOPE,
        session_id,
        serde_json::to_value(&next).unwrap_or(Value::Null),
    )
    .await
    .map_err(|e| ApprovalError::StateUnavailable(format!("settings write failed: {e}")))?;
    Ok(next)
}

/// Drop the stored record (the session reverts to configuration
/// defaults). Returns whether a record existed.
pub async fn clear(iii: &IIIClient, session_id: &str) -> Result<bool, ApprovalError> {
    validate_id("session_id", session_id)?;
    let old = state::delete(iii, SETTINGS_SCOPE, session_id)
        .await
        .map_err(|e| ApprovalError::StateUnavailable(format!("settings delete failed: {e}")))?;
    Ok(!old.is_null())
}

/// Idempotent append keyed on exact `function_id` (prior art:
/// add-always-allow.ts) — returns a new list, never mutates.
pub fn with_grant(
    entries: &[AlwaysAllowEntry],
    function_id: &str,
    granted_at: i64,
) -> Vec<AlwaysAllowEntry> {
    if entries.iter().any(|e| e.function_id == function_id) {
        return entries.to_vec();
    }
    let mut next = entries.to_vec();
    next.push(AlwaysAllowEntry {
        function_id: function_id.to_string(),
        granted_at,
        granted_by: GrantedBy::UserClick,
    });
    next
}

/// Remove by exact `function_id`; absent entries are a no-op. Seed
/// entries are removable like any other — the stored record overrides
/// the deployment seed from first mutation on.
pub fn without_grant(entries: &[AlwaysAllowEntry], function_id: &str) -> Vec<AlwaysAllowEntry> {
    entries
        .iter()
        .filter(|e| e.function_id != function_id)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PermissionMode;
    use serde_json::json;

    fn defaults_with_seed() -> WorkerConfig {
        WorkerConfig {
            default_mode: PermissionMode::Auto,
            rules: vec![
                json!({ "function": "state::get", "action": "allow", "modes": ["auto"] }),
                json!({
                    "function": "engine::functions::list",
                    "action": "allow",
                    "modes": ["auto"]
                }),
            ],
            grant_reask_limit: crate::config::default_grant_reask_limit(),
        }
    }

    #[test]
    fn effective_prefers_stored_record() {
        let stored = ApprovalSettings {
            mode: PermissionMode::Full,
            ..Default::default()
        };
        let (settings, source) = effective(Some(stored.clone()), &defaults_with_seed());
        assert_eq!(settings, stored);
        assert_eq!(source, SettingsSource::Stored);
    }

    #[test]
    fn effective_seeds_from_defaults_when_no_record() {
        let (settings, source) = effective(None, &defaults_with_seed());
        assert_eq!(source, SettingsSource::Defaults);
        assert_eq!(settings.mode, PermissionMode::Auto);
        assert_eq!(settings.always_allow.len(), 2);
        assert!(settings
            .always_allow
            .iter()
            .all(|e| e.granted_by == GrantedBy::Seed));
        assert!(settings.approved_always.is_empty());
    }

    #[test]
    fn parse_settings_rejects_non_objects() {
        assert!(parse_settings(&Value::Null).is_none());
        assert!(parse_settings(&json!("garbage")).is_none());
        assert!(parse_settings(&json!({})).is_some());
    }

    #[test]
    fn with_grant_is_idempotent_on_function_id() {
        let one = with_grant(&[], "shell::run", 10);
        let two = with_grant(&one, "shell::run", 20);
        assert_eq!(one, two);
        assert_eq!(two.len(), 1);
        assert_eq!(two[0].granted_at, 10);
        assert_eq!(two[0].granted_by, GrantedBy::UserClick);
    }

    #[test]
    fn without_grant_removes_and_tolerates_absent() {
        let list = with_grant(&[], "shell::run", 10);
        assert!(without_grant(&list, "shell::run").is_empty());
        assert_eq!(without_grant(&list, "other::fn"), list);
    }
}
