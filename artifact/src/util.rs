//! Small helpers shared by the store, the functions and the share server:
//! the clock, OS-entropy ids and tokens, and RFC 3339 formatting.

use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::store::PasscodeHash;

/// Rounds of SHA-256 over a passcode: slows offline guessing of a stored
/// hash; online guessing is rate-limited by the share server.
const PASSCODE_ROUNDS: u32 = 100_000;

/// Shortest passcode accepted.
pub const MIN_PASSCODE_CHARS: usize = 6;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn digest(salt: &str, passcode: &str) -> String {
    let mut out = Sha256::new()
        .chain_update(salt.as_bytes())
        .chain_update(passcode.as_bytes())
        .finalize();
    for _ in 1..PASSCODE_ROUNDS {
        out = Sha256::new().chain_update(out).finalize();
    }
    hex(&out)
}

/// Hash a passcode with a fresh 128-bit salt.
pub fn hash_passcode(passcode: &str) -> Result<PasscodeHash, String> {
    let mut salt = [0u8; 16];
    getrandom::getrandom(&mut salt).map_err(|e| format!("the OS entropy source failed: {e}"))?;
    let salt = hex(&salt);
    let hash = digest(&salt, passcode);
    Ok(PasscodeHash { salt, hash })
}

/// Check a passcode, comparing in constant time.
pub fn verify_passcode(passcode: &str, stored: &PasscodeHash) -> bool {
    let candidate = digest(&stored.salt, passcode);
    candidate.len() == stored.hash.len()
        && candidate
            .bytes()
            .zip(stored.hash.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// Seconds since the unix epoch.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Bytes of entropy in a share token: 256 bits, so a token cannot be guessed.
const TOKEN_BYTES: usize = 32;

/// Length of a share token in hex characters.
pub const TOKEN_LEN: usize = TOKEN_BYTES * 2;

const ID_LEN: usize = 8;
const ID_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

fn os_random(buf: &mut [u8]) -> Result<(), String> {
    getrandom::getrandom(buf).map_err(|e| format!("the OS entropy source failed: {e}"))
}

/// An 8-character lowercase `[a-z0-9]` artifact id.
pub fn random_id() -> Result<String, String> {
    let mut buf = [0u8; ID_LEN];
    os_random(&mut buf)?;
    Ok(buf
        .iter()
        .map(|b| ID_ALPHABET[usize::from(*b) % ID_ALPHABET.len()] as char)
        .collect())
}

/// A 64-character lowercase hex share token from 256 bits of OS entropy.
pub fn random_token() -> Result<String, String> {
    let mut buf = [0u8; TOKEN_BYTES];
    os_random(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// `true` when `token` has the exact shape [`random_token`] produces. The
/// share server rejects anything else before touching the store.
pub fn is_token_shaped(token: &str) -> bool {
    token.len() == TOKEN_LEN
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Format unix seconds as an RFC 3339 UTC timestamp (`2026-10-01T12:00:00Z`).
pub fn rfc3339(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day), after
/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_formats_known_instants() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_790_000_000), "2026-09-21T14:13:20Z");
        assert_eq!(rfc3339(4_102_444_799), "2099-12-31T23:59:59Z");
    }

    #[test]
    fn tokens_are_64_hex_chars_and_unique() {
        let a = random_token().expect("token");
        let b = random_token().expect("token");
        assert!(is_token_shaped(&a), "{a}");
        assert_ne!(a, b);
    }

    #[test]
    fn malformed_tokens_are_not_token_shaped() {
        assert!(!is_token_shaped(""));
        assert!(!is_token_shaped(&"g".repeat(TOKEN_LEN)));
        assert!(!is_token_shaped(&"A".repeat(TOKEN_LEN)));
        assert!(!is_token_shaped(&"a".repeat(TOKEN_LEN - 1)));
        assert!(!is_token_shaped("../../etc/passwd"));
    }

    #[test]
    fn passcodes_verify_only_with_the_right_secret() {
        let stored = hash_passcode("open sesame").expect("hash");
        assert!(verify_passcode("open sesame", &stored));
        assert!(!verify_passcode("open sesamE", &stored));
        assert!(!verify_passcode("", &stored));
        let again = hash_passcode("open sesame").expect("hash");
        assert_ne!(stored.salt, again.salt, "every hash gets its own salt");
    }

    #[test]
    fn ids_are_8_lowercase_alphanumerics() {
        let id = random_id().expect("id");
        assert_eq!(id.len(), 8);
        assert!(
            id.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
    }
}
