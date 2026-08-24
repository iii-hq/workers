use serde::{Deserialize, Serialize};

use crate::{RunRecordV1, SecurityScanError};

const DEFAULT_PREFIX: &str = "runs/";
const LEGACY_MANIFEST_NAME: &str = "manifest.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchiveIndexRecordV1 {
    pub schema_version: String,
    pub run_id: String,
}

pub(crate) fn index_record(run_id: &str) -> ArchiveIndexRecordV1 {
    ArchiveIndexRecordV1 {
        schema_version: "1".into(),
        run_id: run_id.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct LegacyArchiveManifestV1 {
    pub run_ids: Vec<String>,
}

pub fn object_key(prefix: Option<&str>, run_id: &str) -> Result<String, SecurityScanError> {
    if !is_safe_run_id(run_id) {
        return Err(SecurityScanError::InvalidRequest(format!(
            "archive run id {run_id} is not a safe object key"
        )));
    }
    Ok(format!("{}{run_id}.json", normalize_prefix(prefix)))
}

pub(crate) fn legacy_manifest_key(prefix: Option<&str>) -> String {
    format!("{}{LEGACY_MANIFEST_NAME}", normalize_prefix(prefix))
}

pub fn encode_run(run: &RunRecordV1) -> Result<String, SecurityScanError> {
    encode_json(run, "archived run")
}

pub fn decode_run(body_base64: &str) -> Result<RunRecordV1, SecurityScanError> {
    decode_json(body_base64, "archived run")
}

pub(crate) fn decode_legacy_manifest(
    body_base64: &str,
) -> Result<LegacyArchiveManifestV1, SecurityScanError> {
    decode_json(body_base64, "legacy archive manifest")
}

#[cfg(test)]
pub fn is_run_object_key(prefix: Option<&str>, key: &str) -> bool {
    let prefix = normalize_prefix(prefix);
    key.starts_with(&prefix)
        && key.ends_with(".json")
        && is_safe_run_id(&key[prefix.len()..key.len() - 5])
}

fn encode_json<T: Serialize>(value: &T, label: &str) -> Result<String, SecurityScanError> {
    let body = serde_json::to_vec_pretty(value).map_err(|error| {
        SecurityScanError::Dependency(format!("could not serialize {label}: {error}"))
    })?;
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        body,
    ))
}

fn decode_json<T: for<'de> Deserialize<'de>>(
    body_base64: &str,
    label: &str,
) -> Result<T, SecurityScanError> {
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body_base64)
        .map_err(|error| {
            SecurityScanError::Dependency(format!("{label} is not valid base64: {error}"))
        })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        SecurityScanError::Dependency(format!("{label} is not valid JSON: {error}"))
    })
}

fn normalize_prefix(prefix: Option<&str>) -> String {
    match prefix.map(str::trim).filter(|value| !value.is_empty()) {
        Some(prefix) if prefix.ends_with('/') => prefix.to_string(),
        Some(prefix) => format!("{prefix}/"),
        None => DEFAULT_PREFIX.into(),
    }
}

fn is_safe_run_id(run_id: &str) -> bool {
    !run_id.is_empty()
        && run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        RunStatusV1, ScanModeV1, SecurityAssessmentsV1, SecurityFindingV1, SecurityReportV1,
        SeverityV1,
    };

    fn completed_run() -> RunRecordV1 {
        RunRecordV1 {
            schema_version: "1".into(),
            run_id: "sec_f5fa8c0c2b3e0564ca94cfcb9b2cd0a94dc2b6a491939a865dca93e67de6593e".into(),
            repository: "iii-hq/iii".into(),
            target_sha: "ac636a7e02d9c1beab1ee712accf273674762d75".into(),
            resolved_from_head: false,
            mode: ScanModeV1::Scan,
            model: None,
            provider: None,
            operation_nonce: "nonce".into(),
            status: RunStatusV1::Completed,
            attempt: 6,
            step: 2,
            step_failures: 0,
            materialized: None,
            harness: None,
            report: Some(SecurityReportV1 {
                summary: "Verified three supply-chain weaknesses.".into(),
                assessments: SecurityAssessmentsV1::default(),
                findings: vec![SecurityFindingV1 {
                    rule_id: "SUPPLY_CHAIN_UNSIGNED_RELEASE_ASSETS".into(),
                    severity: SeverityV1::High,
                    title: "Installer executes unverified GitHub release artifacts".into(),
                    description: "The installer downloads release assets without checksums.".into(),
                    evidence: "engine/install.sh".into(),
                    location: None,
                    remediation: "Verify checksums before install.".into(),
                    suggested_patch: None,
                }],
            }),
            error: None,
            created_at: 1,
            updated_at: 2,
            completed_at: Some(2),
        }
    }

    #[test]
    fn object_key_uses_the_configured_prefix() {
        assert_eq!(
            object_key(Some("reports"), "sec_abc").unwrap(),
            "reports/sec_abc.json"
        );
        assert_eq!(object_key(None, "sec_abc").unwrap(), "runs/sec_abc.json");
        assert_eq!(legacy_manifest_key(None), "runs/manifest.json");
    }

    #[test]
    fn encoded_run_round_trips() {
        let run = completed_run();
        let encoded = encode_run(&run).unwrap();
        assert_eq!(decode_run(&encoded).unwrap(), run);
        assert!(is_run_object_key(
            None,
            "runs/sec_f5fa8c0c2b3e0564ca94cfcb9b2cd0a94dc2b6a491939a865dca93e67de6593e.json"
        ));
    }

    #[test]
    fn object_key_rejects_path_escape() {
        assert!(object_key(None, "../escape").is_err());
        assert!(!is_run_object_key(None, "runs/../escape.json"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_archive_membership_uses_independent_run_keys() {
        let mut tasks = Vec::new();
        for index in 0..64 {
            tasks.push(tokio::spawn(async move {
                let run_id = format!("sec_{index:02}");
                let record = index_record(&run_id);
                (run_id, record)
            }));
        }
        let mut keys = std::collections::HashSet::new();
        for task in tasks {
            let (key, record) = task.await.unwrap();
            assert_eq!(record.run_id, key);
            assert!(keys.insert(key), "archive index keys must never collide");
        }
        assert_eq!(keys.len(), 64);
    }
}
