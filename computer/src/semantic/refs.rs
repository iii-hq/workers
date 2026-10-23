//! Element refs: unique for the life of the process and across restarts, so
//! a ref from another session or from before a restart can never resolve to
//! a different element. The counter starts at 48 random bits.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

static NEXT: LazyLock<AtomicU64> =
    LazyLock::new(|| AtomicU64::new(RandomState::new().hash_one(0u8) >> 16));

pub fn next_ref() -> String {
    format!("e{}", base36(NEXT.fetch_add(1, Ordering::Relaxed)))
}

fn base36(mut n: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("base36 digits are ascii")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_are_short_unique_base36() {
        let a = next_ref();
        let b = next_ref();
        assert_ne!(a, b);
        assert!(a.starts_with('e') && a.len() <= 12, "{a}");
        assert!(a[1..]
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()));
        assert_eq!(base36(0), "0");
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
    }
}
