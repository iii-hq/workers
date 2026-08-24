use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{SecurityActionKindV1, SecurityScanRequestV1};

pub fn run_id(request: &SecurityScanRequestV1, model: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"security-scan:profile:v2");
    digest.update([0]);
    digest.update(request.repository.as_bytes());
    digest.update([0]);
    digest.update(request.target_sha.as_bytes());
    digest.update([0]);
    digest.update(request.mode.as_str().as_bytes());
    digest.update([0]);
    digest.update(model.as_bytes());
    let encoded = format!("{:x}", digest.finalize());
    format!("sec_{encoded}")
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

pub fn operation_nonce() -> String {
    Uuid::new_v4().simple().to_string()
}

pub fn action_id(run_id: &str, finding_index: u32, action: SecurityActionKindV1) -> String {
    let mut digest = Sha256::new();
    digest.update(b"security-scan:action:v1");
    digest.update([0]);
    digest.update(run_id.as_bytes());
    digest.update([0]);
    digest.update(finding_index.to_le_bytes());
    digest.update([0]);
    digest.update(action.as_str().as_bytes());
    let encoded = format!("{:x}", digest.finalize());
    format!("seca_{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ScanModeV1;

    #[test]
    fn run_id_changes_when_the_analysis_model_changes() {
        let request = SecurityScanRequestV1::new(
            "iii-hq/iii".into(),
            "0123456789abcdef0123456789abcdef01234567".into(),
            ScanModeV1::Scan,
        );
        let first = run_id(&request, "deepseek::deepseek-v4-flash");
        let second = run_id(&request, "codex/gpt-5.6-terra");
        assert_ne!(first, second);
        assert!(first.starts_with("sec_"));
        assert!(second.starts_with("sec_"));
    }
}
