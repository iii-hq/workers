//! Per-session settings: `approval_settings/<session_id>`, created
//! lazily on first mutation only (approval-gate.md § State lifecycle).
//! Reads never write — the *effective* settings are the stored record
//! when one exists, else the configuration defaults computed in memory.

use serde_json::{json, Value};

use crate::bus::Bus;
use crate::error::ApprovalError;
use crate::gate_config::GateDefaults;
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
/// deployment defaults, seed entries marked `granted_by: "seed"`.
pub fn seeded_from(defaults: &GateDefaults, granted_at: i64) -> ApprovalSettings {
    ApprovalSettings {
        mode: defaults.default_mode,
        always_allow: defaults
            .always_allow_seed
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
    defaults: &GateDefaults,
) -> (ApprovalSettings, SettingsSource) {
    match stored {
        Some(settings) => (settings, SettingsSource::Stored),
        None => (seeded_from(defaults, 0), SettingsSource::Defaults),
    }
}

/// Hot-path read for the gate: any failure (state outage, absent record,
/// garbage) degrades to `None` → configuration defaults. Safe because the
/// default mode never widens beyond what the deployment configured.
pub async fn read_tolerant(
    bus: &dyn Bus,
    session_id: &str,
    timeout_ms: u64,
) -> Option<ApprovalSettings> {
    let reply = bus
        .call(
            "state::get",
            json!({ "scope": SETTINGS_SCOPE, "key": session_id }),
            Some(timeout_ms),
        )
        .await;
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
    bus: &dyn Bus,
    session_id: &str,
    timeout_ms: u64,
) -> Result<Option<ApprovalSettings>, ApprovalError> {
    let reply = bus
        .call(
            "state::get",
            json!({ "scope": SETTINGS_SCOPE, "key": session_id }),
            Some(timeout_ms),
        )
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
    bus: &dyn Bus,
    session_id: &str,
    defaults: &GateDefaults,
    timeout_ms: u64,
    mutate: F,
) -> Result<ApprovalSettings, ApprovalError>
where
    F: FnOnce(ApprovalSettings, i64) -> ApprovalSettings,
{
    validate_id("session_id", session_id)?;
    let now = now_ms();
    let base = read_strict(bus, session_id, timeout_ms)
        .await?
        .unwrap_or_else(|| seeded_from(defaults, now));
    let next = mutate(base, now);
    bus.call(
        "state::set",
        json!({ "scope": SETTINGS_SCOPE, "key": session_id, "value": next }),
        Some(timeout_ms),
    )
    .await
    .map_err(|e| ApprovalError::StateUnavailable(format!("settings write failed: {e}")))?;
    Ok(next)
}

/// Drop the stored record (the session reverts to configuration
/// defaults). Returns whether a record existed.
pub async fn clear(
    bus: &dyn Bus,
    session_id: &str,
    timeout_ms: u64,
) -> Result<bool, ApprovalError> {
    validate_id("session_id", session_id)?;
    let old = bus
        .call(
            "state::delete",
            json!({ "scope": SETTINGS_SCOPE, "key": session_id }),
            Some(timeout_ms),
        )
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
    use crate::testkit::FakeBus;
    use crate::types::PermissionMode;
    use serde_json::json;

    fn defaults_with_seed() -> GateDefaults {
        GateDefaults {
            default_mode: PermissionMode::Auto,
            always_allow_seed: vec!["state::get".into(), "engine::functions::list".into()],
            pending_timeout_ms: 1_800_000,
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

    #[tokio::test]
    async fn reads_never_write() {
        let bus = FakeBus::new();
        let state = bus.with_memory_state();
        let stored = read_tolerant(&bus, "s_1", 100).await;
        assert!(stored.is_none());
        assert!(state.is_empty());
        assert!(bus.calls_to("state::set").is_empty());
    }

    #[tokio::test]
    async fn first_mutation_materializes_from_current_defaults() {
        let bus = FakeBus::new();
        let state = bus.with_memory_state();
        let defaults = defaults_with_seed();

        let next = materialize_and(&bus, "s_1", &defaults, 100, |s, now| ApprovalSettings {
            mode: PermissionMode::Manual,
            mode_set_at: now,
            ..s
        })
        .await
        .unwrap();

        assert_eq!(next.mode, PermissionMode::Manual);
        // Seed copied into the record on first mutation.
        assert_eq!(next.always_allow.len(), 2);
        assert!(next
            .always_allow
            .iter()
            .all(|e| e.granted_by == GrantedBy::Seed));

        let written = state.peek(SETTINGS_SCOPE, "s_1").unwrap();
        let parsed = parse_settings(&written).unwrap();
        assert_eq!(parsed, next);
    }

    #[tokio::test]
    async fn later_seed_changes_do_not_edit_a_stored_record() {
        let bus = FakeBus::new();
        let _state = bus.with_memory_state();
        let defaults = defaults_with_seed();

        materialize_and(&bus, "s_1", &defaults, 100, |s, _| s)
            .await
            .unwrap();

        // Seed changed after the record materialized.
        let changed = GateDefaults {
            always_allow_seed: vec!["everything::*".into()],
            ..defaults_with_seed()
        };
        let next = materialize_and(&bus, "s_1", &changed, 100, |s, _| s)
            .await
            .unwrap();
        assert_eq!(next.always_allow.len(), 2);
        assert!(next
            .always_allow
            .iter()
            .all(|e| e.function_id != "everything::*"));
    }

    #[tokio::test]
    async fn mutation_fails_closed_on_state_outage() {
        let bus = FakeBus::new();
        // No state handlers scripted: every state call errors.
        let err = materialize_and(&bus, "s_1", &GateDefaults::default(), 100, |s, _| s)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "approval/state_unavailable");
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

    #[tokio::test]
    async fn clear_reports_whether_a_record_existed() {
        let bus = FakeBus::new();
        let _state = bus.with_memory_state();
        assert!(!clear(&bus, "s_1", 100).await.unwrap());
        materialize_and(&bus, "s_1", &GateDefaults::default(), 100, |s, _| s)
            .await
            .unwrap();
        assert!(clear(&bus, "s_1", 100).await.unwrap());
        assert!(!clear(&bus, "s_1", 100).await.unwrap());
    }
}
