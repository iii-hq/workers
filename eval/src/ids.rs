use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::contract::VariantRoleV1;

pub fn evaluation_id() -> String {
    format!("eval_{}", Uuid::new_v4().simple())
}

pub fn run_id(role: VariantRoleV1, iteration: u32) -> String {
    format!("{}-{iteration}", role.as_str())
}

pub fn session_id(evaluation_id: &str, role: VariantRoleV1, iteration: u32) -> String {
    let suffix = match role {
        VariantRoleV1::Control => "c",
        VariantRoleV1::Treatment => "t",
    };
    format!("{evaluation_id}_{suffix}_{iteration}")
}

pub fn send_idempotency_key(evaluation_id: &str, role: VariantRoleV1, iteration: u32) -> String {
    format!("eval:{evaluation_id}:{}:{iteration}:send", role.as_str())
}

pub fn finalization_idempotency_key(
    evaluation_id: &str,
    role: VariantRoleV1,
    iteration: u32,
) -> String {
    format!(
        "eval:{evaluation_id}:{}:{iteration}:finalize",
        role.as_str()
    )
}

pub fn sha256_json(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

pub fn sha256_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}
