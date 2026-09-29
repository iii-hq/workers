//! Identifiers and clocks.
//!
//! Ids are prefixed and time-ordered: a UUID v7 rendered without dashes, so
//! sorting by id sorts by creation and a primary-key index is append-friendly
//! instead of scattering writes across the B-tree the way a v4 would.
//!
//! The investigation's session id is derived from the investigation id rather
//! than generated: `sentinel-inv-<id>` is what the ingest tests a trace's
//! `iii.session.id` against to drop the Sentinel's own investigation traces,
//! and what [`crate::functions::DIAGNOSIS_RECORD_ID`] reads back out of the
//! invocation baggage to know which investigation is calling.

use uuid::Uuid;

/// Prefix of every investigation session id. Traces tagged with a session id
/// under this prefix are the Sentinel looking at itself and are never ingested.
pub const INVESTIGATION_SESSION_PREFIX: &str = "sentinel-inv-";

fn ordered() -> String {
    Uuid::now_v7().simple().to_string()
}

pub fn group_id() -> String {
    format!("grp_{}", ordered())
}

pub fn occurrence_id() -> String {
    format!("occ_{}", ordered())
}

pub fn investigation_id() -> String {
    format!("inv_{}", ordered())
}

pub fn transition_id() -> String {
    format!("trn_{}", ordered())
}

pub fn diagnosis_id() -> String {
    format!("dgn_{}", ordered())
}

/// The harness session an investigation runs in.
pub fn investigation_session_id(investigation_id: &str) -> String {
    format!("{INVESTIGATION_SESSION_PREFIX}{investigation_id}")
}

/// Whether a session id names one of this worker's investigations.
pub fn is_investigation_session(session_id: &str) -> bool {
    session_id.starts_with(INVESTIGATION_SESSION_PREFIX)
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

/// The hour a timestamp falls in, as epoch milliseconds — the bucket key that
/// keeps the sparkline off a scan of the occurrence table.
pub fn hour_bucket_ms(at_ms: i64) -> i64 {
    const HOUR_MS: i64 = 3_600_000;
    at_ms.div_euclid(HOUR_MS) * HOUR_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_carry_their_prefix_and_sort_by_creation() {
        let first = group_id();
        let second = group_id();
        assert!(first.starts_with("grp_"));
        assert!(second.starts_with("grp_"));
        assert_ne!(first, second);
        assert!(first < second, "{first} should sort before {second}");

        assert!(occurrence_id().starts_with("occ_"));
        assert!(investigation_id().starts_with("inv_"));
        assert!(diagnosis_id().starts_with("dgn_"));
    }

    #[test]
    fn investigation_sessions_are_recognisable_from_their_id_alone() {
        let investigation = investigation_id();
        let session = investigation_session_id(&investigation);
        assert!(session.starts_with(INVESTIGATION_SESSION_PREFIX));
        assert!(is_investigation_session(&session));
        assert!(!is_investigation_session("console-42"));
    }

    #[test]
    fn hour_buckets_round_down_on_both_sides_of_the_epoch() {
        assert_eq!(hour_bucket_ms(3_600_000), 3_600_000);
        assert_eq!(hour_bucket_ms(3_600_001), 3_600_000);
        assert_eq!(hour_bucket_ms(7_199_999), 3_600_000);
        assert_eq!(hour_bucket_ms(-1), -3_600_000);
    }
}
