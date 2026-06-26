//! Slack request signature verification.
//!
//! Slack signs each request as `v0={hex hmac_sha256(signing_secret,
//! "v0:{timestamp}:{raw_body}")}` in the `X-Slack-Signature` header, with the
//! unix `X-Slack-Request-Timestamp`. We verify over the **raw** request body
//! (read from the engine's `request_body` channel) in constant time and reject
//! stale timestamps to block replay.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const MAX_SKEW_SECS: i64 = 60 * 5;

/// Verify a Slack signature. `now_unix` is the current unix time (seconds).
pub fn verify(
    signing_secret: &str,
    signature: Option<&str>,
    timestamp: Option<&str>,
    raw_body: &[u8],
    now_unix: i64,
) -> Result<(), String> {
    let signature = signature.ok_or("missing X-Slack-Signature")?;
    let ts_str = timestamp.ok_or("missing X-Slack-Request-Timestamp")?;
    let ts: i64 = ts_str
        .trim()
        .parse()
        .map_err(|_| "invalid X-Slack-Request-Timestamp".to_string())?;
    if (now_unix - ts).abs() > MAX_SKEW_SECS {
        return Err("stale request timestamp (replay window exceeded)".into());
    }

    let expected_hex = format!("v0={}", sign_hex(signing_secret, ts_str.trim(), raw_body)?);
    if constant_time_eq(expected_hex.as_bytes(), signature.as_bytes()) {
        Ok(())
    } else {
        Err("signature mismatch".into())
    }
}

/// Compute the hex hmac for `v0:{timestamp}:{body}` (without the `v0=` prefix).
fn sign_hex(signing_secret: &str, timestamp: &str, raw_body: &[u8]) -> Result<String, String> {
    let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes())
        .map_err(|e| format!("hmac key: {e}"))?;
    mac.update(b"v0:");
    mac.update(timestamp.as_bytes());
    mac.update(b":");
    mac.update(raw_body);
    Ok(hex_encode(&mac.finalize().into_bytes()))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig_for(key: &str, ts: &str, body: &str) -> String {
        format!("v0={}", sign_hex(key, ts, body.as_bytes()).unwrap())
    }

    #[test]
    fn accepts_valid_signature() {
        let key = "test-signing-key";
        let ts = "1531420618";
        let body = "token=xyz&team_id=T1";
        let sig = sig_for(key, ts, body);
        assert!(verify(key, Some(&sig), Some(ts), body.as_bytes(), 1531420618).is_ok());
    }

    #[test]
    fn rejects_tampered_body() {
        let key = "k";
        let ts = "1000";
        let sig = sig_for(key, ts, "a=1");
        assert!(verify(key, Some(&sig), Some(ts), b"a=2", 1000).is_err());
    }

    #[test]
    fn rejects_stale_timestamp() {
        let key = "k";
        let ts = "1000";
        let sig = sig_for(key, ts, "a=1");
        assert!(verify(key, Some(&sig), Some(ts), b"a=1", 1000 + 10_000).is_err());
    }

    #[test]
    fn rejects_missing_headers() {
        assert!(verify("k", None, Some("1"), b"x", 1).is_err());
        assert!(verify("k", Some("v0=x"), None, b"x", 1).is_err());
    }
}
