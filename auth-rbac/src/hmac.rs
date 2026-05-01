//! HMAC-SHA-256 token hashing + constant-time hex compare.
//! Direct port of roster/workers/auth/src/hmac.ts.

use base64::Engine;
// Use leading `::` to reference the external `hmac` crate, not this module.
use ::hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

pub const HASH_PREFIX_LEN: usize = 12;

const SECRET_ENV: &str = "AUTH_HMAC_SECRET";

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error(
        "auth-rbac refuses to start: AUTH_HMAC_SECRET env var is not set. \
         Generate one with `openssl rand -hex 32` and export it."
    )]
    Missing,
}

pub fn load_secret() -> Result<String, SecretError> {
    match std::env::var(SECRET_ENV) {
        Ok(s) if !s.is_empty() => Ok(s),
        _ => Err(SecretError::Missing),
    }
}

pub fn hash_token(secret: &str, token: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC-SHA256 accepts any byte length");
    mac.update(token.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn hash_prefix(hash: &str) -> &str {
    let cap = hash.len().min(HASH_PREFIX_LEN);
    &hash[..cap]
}

/// Constant-time hex-string equality. Mirrors Roster's `timingSafeHexEqual`:
/// rejects empty strings, length mismatches, odd-length hex, and non-hex chars
/// before doing the constant-time byte compare.
pub fn timing_safe_hex_equal(a: &str, b: &str) -> bool {
    if a.len() != b.len() || a.is_empty() || !a.len().is_multiple_of(2) {
        return false;
    }
    if !a.chars().all(|c| c.is_ascii_hexdigit()) || !b.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    let Ok(ab) = hex::decode(a) else { return false };
    let Ok(bb) = hex::decode(b) else { return false };
    if ab.len() != bb.len() || ab.is_empty() {
        return false;
    }
    ab.ct_eq(&bb).into()
}

/// Generate `rsk_<workspace_prefix>_<base64url(24 random bytes)>`.
pub fn generate_token(workspace_id: &str) -> String {
    let prefix_len = workspace_id.len().min(8);
    let prefix = &workspace_id[..prefix_len];
    let mut buf = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut buf);
    let body = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf);
    format!("rsk_{prefix}_{body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_token_is_deterministic() {
        assert_eq!(hash_token("secret", "token"), hash_token("secret", "token"));
    }

    #[test]
    fn hash_token_differs_per_secret() {
        assert_ne!(hash_token("a", "token"), hash_token("b", "token"));
    }

    #[test]
    fn hash_token_byte_equiv_to_node_crypto() {
        // Pinned against `printf token | openssl dgst -sha256 -hmac secret -hex`,
        // equivalent to Node's
        // `crypto.createHmac('sha256','secret').update('token').digest('hex')`.
        // A future crate bump that diverges from the Roster implementation
        // must fail this test.
        let expected = "e941110e3d2bfe82621f0e3e1434730d7305d106c5f68c87165d0b27a4611a4a";
        assert_eq!(hash_token("secret", "token"), expected);
    }

    #[test]
    fn timing_safe_hex_equal_accepts_match() {
        assert!(timing_safe_hex_equal("deadbeef", "deadbeef"));
    }

    #[test]
    fn timing_safe_hex_equal_rejects_length_mismatch() {
        assert!(!timing_safe_hex_equal("deadbeef", "deadbeefcafe"));
    }

    #[test]
    fn timing_safe_hex_equal_rejects_non_hex() {
        assert!(!timing_safe_hex_equal("dead!eef", "deadbeef"));
    }

    #[test]
    fn timing_safe_hex_equal_rejects_empty() {
        assert!(!timing_safe_hex_equal("", ""));
    }

    #[test]
    fn timing_safe_hex_equal_rejects_odd_length() {
        assert!(!timing_safe_hex_equal("dea", "dea"));
    }

    #[test]
    fn generate_token_uses_workspace_prefix() {
        let t = generate_token("ws-12345abc");
        assert!(t.starts_with("rsk_ws-12345"), "got {t}");
    }

    #[test]
    fn generate_token_short_workspace_id() {
        let t = generate_token("ab");
        assert!(t.starts_with("rsk_ab_"), "got {t}");
    }
}
