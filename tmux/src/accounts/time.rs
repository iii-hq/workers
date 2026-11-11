//! RFC 3339 timestamps as Claude Code's usage endpoint and Codex's records
//! write them, to milliseconds since the epoch.

/// `2026-11-17T19:00:00.129271+00:00`, `2026-10-23T14:26:30.073Z`.
pub fn parse_rfc3339_ms(text: &str) -> Option<u64> {
    let text = text.trim();
    let bytes = text.as_bytes();
    if bytes.len() < 19 || !matches!(bytes[10], b'T' | b't' | b' ') {
        return None;
    }
    let num = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = text.get(range)?;
        if !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    if bytes[4] != b'-' || bytes[7] != b'-' || bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    let mut rest = &text[19..];
    let mut millis = 0_i64;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits: String = fraction.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            return None;
        }
        let padded: String = digits.chars().chain("000".chars()).take(3).collect();
        millis = padded.parse().ok()?;
        rest = &fraction[digits.len()..];
    }
    let offset_minutes = match rest {
        "Z" | "z" | "" => 0,
        _ => {
            let sign = match rest.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let zone = &rest[1..];
            let (h, m) = zone
                .split_once(':')
                .unwrap_or((zone.get(..2)?, zone.get(2..)?));
            sign * (h.parse::<i64>().ok()? * 60 + m.parse::<i64>().ok()?)
        }
    };
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second - offset_minutes * 60;
    u64::try_from(seconds * 1_000 + millis).ok()
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// A JSON time: an RFC 3339 string, or a number of seconds (or of
/// milliseconds, past the year 33658 in seconds).
pub fn json_time_ms(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::String(text) => parse_rfc3339_ms(text),
        serde_json::Value::Number(number) => {
            let raw = number.as_f64()?;
            if raw <= 0.0 {
                return None;
            }
            Some(if raw >= 1e12 {
                raw as u64
            } else {
                (raw * 1000.0) as u64
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_parse_to_epoch_millis() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_ms("2026-10-23T14:26:30.073Z"),
            Some(1_792_765_590_073)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-11-17T19:00:00.129271+00:00"),
            Some(1_794_942_000_129)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-11-17T16:00:00-03:00"),
            parse_rfc3339_ms("2026-11-17T19:00:00Z")
        );
        assert_eq!(
            parse_rfc3339_ms("2024-02-29T00:00:00Z"),
            Some(1_709_164_800_000)
        );
        assert_eq!(parse_rfc3339_ms("nonsense"), None);
        assert_eq!(parse_rfc3339_ms("2026-13-01T00:00:00Z"), None);
    }

    #[test]
    fn json_times_take_strings_seconds_and_millis() {
        assert_eq!(
            json_time_ms(&serde_json::json!(1_793_303_337)),
            Some(1_793_303_337_000)
        );
        assert_eq!(
            json_time_ms(&serde_json::json!(1_793_303_337_000_u64)),
            Some(1_793_303_337_000)
        );
        assert_eq!(
            json_time_ms(&serde_json::json!("1970-01-01T00:00:01Z")),
            Some(1_000)
        );
        assert_eq!(json_time_ms(&serde_json::Value::Null), None);
    }
}
