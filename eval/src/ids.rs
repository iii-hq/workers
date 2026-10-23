use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Prefix of every investigation session the monitor creates. Recursion
/// guard only — it is not an access check.
pub const ANALYST_PREFIX: &str = "eval_monitor_";

pub fn evaluation_id() -> String {
    format!("eval_{}", Uuid::new_v4().simple())
}

/// Stable identity of one observed session turn. The length prefix keeps the
/// two parts unambiguous.
pub fn observation_key(session_id: &str, turn_id: &str) -> String {
    sha256_text(&format!("{}:{session_id}{turn_id}", session_id.len()))
}

pub fn analyst_session(evaluation_id: &str) -> String {
    format!("{ANALYST_PREFIX}{evaluation_id}")
}

pub fn judge_request_id(evaluation_id: &str, step: u64) -> String {
    format!("{evaluation_id}-judge-{step}")
}

pub fn sha256_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn sha256_json(value: &impl serde::Serialize) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// RFC 3339 to epoch milliseconds (`Z` or a numeric offset, optional
/// fraction): the E2E writes `...Z`, `...123Z` and `...123456789+00:00`, which
/// do not order correctly as text.
pub fn parse_ms(text: &str) -> Option<i64> {
    let (date, rest) = text.split_once('T')?;
    let (clock, offset_minutes) = match rest.strip_suffix('Z') {
        Some(clock) => (clock, 0),
        None => {
            let at = rest.rfind(['+', '-'])?;
            let (clock, offset) = rest.split_at(at);
            let (hours, minutes) = offset[1..].split_once(':')?;
            let minutes = hours.parse::<i64>().ok()? * 60 + minutes.parse::<i64>().ok()?;
            (
                clock,
                if offset.starts_with('-') {
                    -minutes
                } else {
                    minutes
                },
            )
        }
    };
    let mut date = date.split('-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let (clock, fraction) = clock.split_once('.').unwrap_or((clock, ""));
    let mut clock = clock.split(':').map(|part| part.parse::<i64>().ok());
    let (hour, minute, second) = (clock.next()??, clock.next()??, clock.next()??);
    if !(1..=12).contains(&month) || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let millis: i64 = format!("{:0<3}", &fraction[..fraction.len().min(3)])
        .parse()
        .ok()?;
    // Days since 1970-01-01 (proleptic Gregorian, Hinnant's civil algorithm).
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some((((days * 24 + hour) * 60 + minute - offset_minutes) * 60 + second) * 1_000 + millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_key_is_stable_and_unambiguous() {
        assert_eq!(observation_key("s", "t"), observation_key("s", "t"));
        assert_ne!(observation_key("ab", "c"), observation_key("a", "bc"));
        assert_ne!(evaluation_id(), evaluation_id());
    }

    #[test]
    fn timestamps_in_the_e2e_formats_order_by_instant() {
        assert_eq!(parse_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_ms("2000-03-01T00:00:00Z"), Some(951_868_800_000));
        assert_eq!(parse_ms("2024-02-29T12:00:00Z"), Some(1_709_208_000_000));
        assert_eq!(parse_ms("2026-10-02T14:33:13Z"), Some(1_790_951_593_000));
        assert_eq!(
            parse_ms("2026-10-02T14:33:13.182708291+00:00"),
            Some(1_790_951_593_182)
        );
        assert_eq!(
            parse_ms("2026-10-02T10:33:13-04:00"),
            Some(1_790_951_593_000)
        );
        assert_eq!(parse_ms("2026-10-02T14:33:13.5Z"), Some(1_790_951_593_500));
        // As text "…01Z" sorts after "…01.5Z"; as instants it is earlier.
        assert!(parse_ms("2026-10-02T05:17:01Z") < parse_ms("2026-10-02T05:17:01.5Z"));
        assert_eq!(parse_ms(""), None);
        assert_eq!(parse_ms("2026-13-02T00:00:00Z"), None);
        assert_eq!(parse_ms("yesterday"), None);
    }
}
