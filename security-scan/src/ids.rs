use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::SecurityScanRequestV1;

pub fn run_id(request: &SecurityScanRequestV1) -> String {
    let mut digest = Sha256::new();
    digest.update(b"security-scan:profile:v1");
    digest.update([0]);
    digest.update(request.repository.as_bytes());
    digest.update([0]);
    digest.update(request.target_sha.as_bytes());
    digest.update([0]);
    digest.update(request.mode.as_str().as_bytes());
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
