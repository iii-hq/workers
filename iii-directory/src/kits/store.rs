//! Pending plans on disk: `<project>/.iii/directory/kit-plans/<plan_id>.json`,
//! valid for 24 hours. Each record holds the plan, the file bodies it refers
//! to, and the apply journal that makes a retried apply converge.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::plan::{Plan, PlanContents};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApplyStep {
    /// `workers` | `files` | `lock`.
    pub id: String,
    pub label: String,
    pub state: StepState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// What an apply has done so far for this plan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApplyProgress {
    pub steps: Vec<ApplyStep>,
    /// Install path → sha written (or `""` for a deletion) by an earlier,
    /// interrupted attempt. Re-validation accepts these as expected.
    #[serde(default)]
    pub written: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlanRecord {
    pub plan: Plan,
    #[serde(default)]
    pub contents: PlanContents,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<ApplyProgress>,
}

fn valid_id(plan_id: &str) -> bool {
    plan_id.starts_with("kp_")
        && plan_id.len() <= 64
        && plan_id[3..].chars().all(|c| c.is_ascii_alphanumeric())
}

fn record_path(dir: &Path, plan_id: &str) -> Option<PathBuf> {
    valid_id(plan_id).then(|| dir.join(format!("{plan_id}.json")))
}

pub fn save(dir: &Path, record: &PlanRecord) -> Result<(), String> {
    let path = record_path(dir, &record.plan.plan_id)
        .ok_or_else(|| format!("invalid plan id {:?}", record.plan.plan_id))?;
    let json = serde_json::to_vec(record).map_err(|e| format!("encode plan: {e}"))?;
    crate::sources::write_file_atomic(&path, &json)
}

/// Load a plan; `Ok(None)` when it does not exist.
pub fn load(dir: &Path, plan_id: &str) -> Result<Option<PlanRecord>, String> {
    let Some(path) = record_path(dir, plan_id) else {
        return Ok(None);
    };
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("plan {plan_id} is unreadable: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read {}: {e}", path.display())),
    }
}

pub fn delete(dir: &Path, plan_id: &str) -> Result<bool, String> {
    let Some(path) = record_path(dir, plan_id) else {
        return Ok(false);
    };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("delete {}: {e}", path.display())),
    }
}

pub fn is_expired(plan: &Plan, now: chrono::DateTime<chrono::Utc>) -> bool {
    chrono::DateTime::parse_from_rfc3339(&plan.expires_at)
        .map(|t| t.with_timezone(&chrono::Utc) <= now)
        .unwrap_or(true)
}

/// Every pending, unexpired plan (expired ones are deleted on the way).
pub fn list(dir: &Path, now: chrono::DateTime<chrono::Utc>) -> Vec<PlanRecord> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        match serde_json::from_slice::<PlanRecord>(&bytes) {
            Ok(record) if is_expired(&record.plan, now) => {
                let _ = std::fs::remove_file(&path);
            }
            Ok(record) => out.push(record),
            Err(_) => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    out.sort_by(|a, b| b.plan.created_at.cmp(&a.plan.created_at));
    out
}

/// Pending plans for one kit.
pub fn for_kit(dir: &Path, kit: &str, now: chrono::DateTime<chrono::Utc>) -> Vec<PlanRecord> {
    list(dir, now)
        .into_iter()
        .filter(|r| r.plan.kit == kit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kits::plan::{PlanCounts, PlanKind};
    use crate::kits::registry::Capabilities;

    pub(crate) fn plan_at(id: &str, kit: &str, created: chrono::DateTime<chrono::Utc>) -> Plan {
        Plan {
            plan_id: id.into(),
            kind: PlanKind::Install,
            kit: kit.into(),
            from: None,
            to: Some("1.0.0".into()),
            requested: "latest".into(),
            bump: None,
            major: false,
            author: None,
            description: None,
            license: None,
            repo: None,
            published_at: None,
            registry_url: String::new(),
            notes: None,
            deprecation: None,
            workers: vec![],
            files: vec![],
            capabilities: Capabilities::default(),
            functions: vec![],
            warnings: vec![],
            blocking: vec![],
            counts: PlanCounts::default(),
            created_at: created.to_rfc3339(),
            expires_at: (created + chrono::Duration::hours(24)).to_rfc3339(),
            review: String::new(),
        }
    }

    #[test]
    fn plans_round_trip_and_expire_after_a_day() {
        let tmp = tempfile::tempdir().unwrap();
        let now = chrono::Utc::now();
        let fresh = PlanRecord {
            plan: plan_at("kp_fresh", "acme/a", now),
            contents: PlanContents::default(),
            progress: None,
        };
        let stale = PlanRecord {
            plan: plan_at("kp_stale", "acme/b", now - chrono::Duration::hours(25)),
            contents: PlanContents::default(),
            progress: None,
        };
        save(tmp.path(), &fresh).unwrap();
        save(tmp.path(), &stale).unwrap();
        assert_eq!(load(tmp.path(), "kp_fresh").unwrap().unwrap(), fresh);
        let listed = list(tmp.path(), now);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].plan.plan_id, "kp_fresh");
        // The expired plan was cleaned up.
        assert!(load(tmp.path(), "kp_stale").unwrap().is_none());
        assert!(delete(tmp.path(), "kp_fresh").unwrap());
        assert!(!delete(tmp.path(), "kp_fresh").unwrap());
    }

    #[test]
    fn plan_ids_cannot_escape_the_directory() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load(tmp.path(), "../etc/passwd").unwrap().is_none());
        assert!(load(tmp.path(), "kp_../../x").unwrap().is_none());
    }
}
