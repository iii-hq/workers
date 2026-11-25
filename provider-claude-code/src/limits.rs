//! A 429 that means the account's usage limit is reached, told apart from a
//! busy 429 (chat-projects D132, D147).
//!
//! The subscription backend answers OAuth requests with unified rate-limit
//! headers. Anthropic's public API docs cover only the per-key
//! `anthropic-ratelimit-*` headers, so these are read the way Claude Code
//! itself reads them (2.1.332):
//! - `anthropic-ratelimit-unified-status`: `allowed`, `allowed_warning`, or
//!   `rejected` once the account's limit is reached;
//! - `anthropic-ratelimit-unified-representative-claim`: the limit that
//!   decided it, `five_hour` or a weekly one (`seven_day`, `seven_day_opus`,
//!   `seven_day_sonnet`, `seven_day_overage_included`), or `overage`;
//! - `anthropic-ratelimit-unified-reset`: when that limit resets, in unix
//!   seconds;
//! - `anthropic-ratelimit-unified-5h-reset` / `-7d-reset`: each window's own
//!   reset, in unix seconds.
//!
//! A 429 with `rejected` is the limit: `rate_limited` plus `error_details
//! {usage_limit: true, window?, resets_at_ms?}`, which the router and the
//! harness do not retry. Any other 429 stays a plain, retried `rate_limited`.
use llm_router::types::events::{ErrorDetails, UsageWindow};
use reqwest::header::HeaderMap;

pub const UNIFIED_STATUS: &str = "anthropic-ratelimit-unified-status";
pub const UNIFIED_CLAIM: &str = "anthropic-ratelimit-unified-representative-claim";
pub const UNIFIED_RESET: &str = "anthropic-ratelimit-unified-reset";
pub const UNIFIED_FIVE_HOUR_RESET: &str = "anthropic-ratelimit-unified-5h-reset";
pub const UNIFIED_WEEK_RESET: &str = "anthropic-ratelimit-unified-7d-reset";

/// The status Claude Code reads as "the limit is reached".
const REJECTED: &str = "rejected";

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)?
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// The predicate: a 429 whose unified status says the limit is reached.
pub fn is_usage_limit(status: u16, headers: &HeaderMap) -> bool {
    status == 429
        && header(headers, UNIFIED_STATUS).is_some_and(|value| value.eq_ignore_ascii_case(REJECTED))
}

/// The window a representative claim belongs to; `overage` and claims this
/// provider does not know name none.
fn window_of(claim: &str) -> Option<UsageWindow> {
    match claim {
        "five_hour" => Some(UsageWindow::FiveHour),
        claim if claim.starts_with("seven_day") => Some(UsageWindow::Week),
        _ => None,
    }
}

/// Unix seconds (possibly fractional, as Claude Code rounds them) →
/// milliseconds; nothing for a value that is not a positive time.
fn unix_seconds_to_ms(value: &str) -> Option<u64> {
    let seconds: f64 = value.parse().ok()?;
    (seconds.is_finite() && seconds > 0.0).then(|| (seconds * 1000.0).round() as u64)
}

/// The details of a usage-limit 429, or nothing for any other response. The
/// reset is the decisive limit's (`…-reset`), else its window's own.
pub fn usage_limit_details(status: u16, headers: &HeaderMap) -> Option<ErrorDetails> {
    if !is_usage_limit(status, headers) {
        return None;
    }
    let window = header(headers, UNIFIED_CLAIM).and_then(window_of);
    let window_reset = match window {
        Some(UsageWindow::FiveHour) => header(headers, UNIFIED_FIVE_HOUR_RESET),
        Some(UsageWindow::Week) => header(headers, UNIFIED_WEEK_RESET),
        None => None,
    };
    let resets_at_ms = header(headers, UNIFIED_RESET)
        .and_then(unix_seconds_to_ms)
        .or_else(|| window_reset.and_then(unix_seconds_to_ms));
    Some(ErrorDetails::usage_limit(window, resets_at_ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn only_a_429_with_a_rejected_unified_status_is_the_limit() {
        let rejected = headers(&[(UNIFIED_STATUS, "rejected")]);
        assert!(is_usage_limit(429, &rejected));
        assert!(is_usage_limit(
            429,
            &headers(&[(UNIFIED_STATUS, " Rejected ")])
        ));
        // A busy 429: no unified status, or one that still allows requests.
        assert!(!is_usage_limit(429, &HeaderMap::new()));
        assert!(!is_usage_limit(
            429,
            &headers(&[(UNIFIED_STATUS, "allowed")])
        ));
        assert!(!is_usage_limit(
            429,
            &headers(&[(UNIFIED_STATUS, "allowed_warning")])
        ));
        assert!(!is_usage_limit(429, &headers(&[(UNIFIED_STATUS, "")])));
        // The status alone, on another response, is not a failure.
        assert!(!is_usage_limit(200, &rejected));
        assert!(!is_usage_limit(529, &rejected));
    }

    #[test]
    fn the_five_hour_limit_with_its_reset_in_milliseconds() {
        let details = usage_limit_details(
            429,
            &headers(&[
                (UNIFIED_STATUS, "rejected"),
                (UNIFIED_CLAIM, "five_hour"),
                (UNIFIED_RESET, "1800000000"),
                (UNIFIED_FIVE_HOUR_RESET, "1700000000"),
            ]),
        );
        assert_eq!(
            details,
            Some(ErrorDetails::usage_limit(
                Some(UsageWindow::FiveHour),
                Some(1_800_000_000_000)
            ))
        );
    }

    #[test]
    fn every_seven_day_claim_is_the_week() {
        for claim in [
            "seven_day",
            "seven_day_opus",
            "seven_day_sonnet",
            "seven_day_overage_included",
        ] {
            let details = usage_limit_details(
                429,
                &headers(&[(UNIFIED_STATUS, "rejected"), (UNIFIED_CLAIM, claim)]),
            )
            .unwrap();
            assert_eq!(details.window, Some(UsageWindow::Week), "{claim}");
            assert!(details.usage_limit);
            assert_eq!(details.resets_at_ms, None);
        }
    }

    #[test]
    fn without_the_overall_reset_the_windows_own_is_used() {
        let details = usage_limit_details(
            429,
            &headers(&[
                (UNIFIED_STATUS, "rejected"),
                (UNIFIED_CLAIM, "seven_day"),
                (UNIFIED_FIVE_HOUR_RESET, "1700000000"),
                (UNIFIED_WEEK_RESET, "1800000000.4"),
            ]),
        )
        .unwrap();
        assert_eq!(details.resets_at_ms, Some(1_800_000_000_400));
    }

    #[test]
    fn an_unknown_claim_or_a_bad_reset_leaves_only_the_flag() {
        let details = usage_limit_details(
            429,
            &headers(&[
                (UNIFIED_STATUS, "rejected"),
                (UNIFIED_CLAIM, "overage"),
                (UNIFIED_RESET, "soon"),
            ]),
        );
        assert_eq!(details, Some(ErrorDetails::usage_limit(None, None)));
        let negative = usage_limit_details(
            429,
            &headers(&[(UNIFIED_STATUS, "rejected"), (UNIFIED_RESET, "-5")]),
        );
        assert_eq!(negative, Some(ErrorDetails::usage_limit(None, None)));
    }

    #[test]
    fn a_busy_429_has_no_details() {
        assert_eq!(usage_limit_details(429, &HeaderMap::new()), None);
        assert_eq!(
            usage_limit_details(
                429,
                &headers(&[
                    (UNIFIED_STATUS, "allowed_warning"),
                    (UNIFIED_RESET, "1800000000")
                ])
            ),
            None
        );
    }
}
