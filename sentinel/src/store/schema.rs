//! The store's shape, and how it moves forward.
//!
//! Tables are created with `IF NOT EXISTS` at boot and versioned by a row in
//! `sentinel_meta`, so a fresh install and an upgrade run the same path. Each
//! version is one atomic batch ending in the version bump: a migration that
//! dies halfway leaves the previous version in place rather than a half-built
//! schema.
//!
//! Two shapes here are load-bearing and easy to get wrong:
//!
//! - `sentinel_occurrences.group_id` is **nullable, but only while
//!   `pending_join = 1`**. An ERROR log arrives before the span that explains
//!   it, and the group of a log-sourced failure is only created if the wait
//!   times out — so the row has to exist before its group does. The CHECK is
//!   what keeps that from becoming "nulls happen sometimes".
//! - The decision-tier tables are created empty. They cost nothing, and
//!   having them from the start means the optional tier is a configuration
//!   change later rather than a migration.

/// The schema version this build expects.
pub const SCHEMA_VERSION: u32 = 1;

pub const META_TABLE: &str =
    "CREATE TABLE IF NOT EXISTS sentinel_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)";

pub const SCHEMA_VERSION_KEY: &str = "schema_version";

/// Statements for one version, applied in order inside a single transaction.
pub fn migrations() -> Vec<(u32, Vec<&'static str>)> {
    vec![(1, v1())]
}

fn v1() -> Vec<&'static str> {
    vec![
        // ── groups ───────────────────────────────────────────────────────
        r#"CREATE TABLE IF NOT EXISTS sentinel_groups (
  id                TEXT PRIMARY KEY,
  fingerprint       TEXT NOT NULL UNIQUE,
  source            TEXT NOT NULL,
  namespace         TEXT NOT NULL DEFAULT 'default',
  service_name      TEXT NOT NULL,
  function_id       TEXT,
  exception_type    TEXT,
  title             TEXT NOT NULL,
  summary           TEXT,
  triage            TEXT,
  message_sample    TEXT NOT NULL,
  status            TEXT NOT NULL,
  previous_status   TEXT,
  ignore_rule       TEXT,
  ignore_baseline   TEXT,
  first_seen_ms     INTEGER NOT NULL,
  last_seen_ms      INTEGER NOT NULL,
  occurrence_count  INTEGER NOT NULL DEFAULT 0,
  first_version     TEXT,
  last_version      TEXT,
  resolved_at_ms    INTEGER,
  resolved_version  TEXT,
  resolve_until_version_change INTEGER NOT NULL DEFAULT 0,
  regressed_at_ms   INTEGER,
  diagnosis_id      TEXT,
  archived          INTEGER NOT NULL DEFAULT 0,
  updated_ms        INTEGER NOT NULL
)"#,
        "CREATE INDEX IF NOT EXISTS sentinel_groups_list ON sentinel_groups (archived, status, last_seen_ms DESC)",
        "CREATE INDEX IF NOT EXISTS sentinel_groups_service ON sentinel_groups (service_name, last_seen_ms DESC)",
        // ── occurrences ──────────────────────────────────────────────────
        r#"CREATE TABLE IF NOT EXISTS sentinel_occurrences (
  id              TEXT PRIMARY KEY,
  group_id        TEXT REFERENCES sentinel_groups(id),
  dedupe_key      TEXT NOT NULL UNIQUE,
  source          TEXT NOT NULL,
  at_ms           INTEGER NOT NULL,
  trace_id        TEXT,
  span_id         TEXT,
  session_id      TEXT,
  turn_id         TEXT,
  worker_version  TEXT,
  message         TEXT NOT NULL,
  evidence        TEXT,
  evidence_bytes  INTEGER NOT NULL DEFAULT 0,
  settled         INTEGER NOT NULL DEFAULT 0,
  pending_join    INTEGER NOT NULL DEFAULT 0,
  join_deadline_ms INTEGER,
  session_unknown INTEGER NOT NULL DEFAULT 0,
  namespace_ambiguous INTEGER NOT NULL DEFAULT 0,
  novelty         REAL,
  membership_doubt REAL,
  CHECK ((pending_join = 1 AND group_id IS NULL) OR (pending_join = 0 AND group_id IS NOT NULL))
)"#,
        "CREATE INDEX IF NOT EXISTS sentinel_occurrences_group ON sentinel_occurrences (group_id, at_ms DESC)",
        "CREATE INDEX IF NOT EXISTS sentinel_occurrences_trace ON sentinel_occurrences (trace_id)",
        "CREATE INDEX IF NOT EXISTS sentinel_occurrences_pending ON sentinel_occurrences (pending_join, join_deadline_ms)",
        // ── sessions affected ────────────────────────────────────────────
        // Never pruned with the occurrences: this is what keeps "sessions
        // affected" true after the 1000-row cap starts cutting.
        r#"CREATE TABLE IF NOT EXISTS sentinel_group_sessions (
  group_id     TEXT NOT NULL REFERENCES sentinel_groups(id),
  session_id   TEXT NOT NULL,
  first_ms     INTEGER NOT NULL,
  last_ms      INTEGER NOT NULL,
  PRIMARY KEY (group_id, session_id)
)"#,
        // ── hourly buckets, for the sparkline ────────────────────────────
        r#"CREATE TABLE IF NOT EXISTS sentinel_buckets (
  group_id  TEXT NOT NULL,
  hour_ms   INTEGER NOT NULL,
  count     INTEGER NOT NULL,
  PRIMARY KEY (group_id, hour_ms)
)"#,
        // ── investigations ───────────────────────────────────────────────
        r#"CREATE TABLE IF NOT EXISTS sentinel_investigations (
  id               TEXT PRIMARY KEY,
  group_id         TEXT NOT NULL REFERENCES sentinel_groups(id),
  occurrence_id    TEXT NOT NULL,
  session_id       TEXT NOT NULL,
  mode             TEXT NOT NULL,
  first_pass_turn_id TEXT,
  nudged           INTEGER NOT NULL DEFAULT 0,
  model            TEXT NOT NULL,
  provider         TEXT,
  repository_id    TEXT,
  checkout_ref     TEXT,
  investigated_version TEXT,
  status           TEXT NOT NULL,
  error            TEXT,
  turns            INTEGER,
  duration_ms      INTEGER,
  cost_usd         REAL,
  requested_by     TEXT,
  created_ms       INTEGER NOT NULL,
  finished_ms      INTEGER
)"#,
        // One running first pass per group — the durable guard behind
        // `investigate` returning the existing session instead of a second.
        "CREATE UNIQUE INDEX IF NOT EXISTS sentinel_investigations_active ON sentinel_investigations (group_id) WHERE status = 'running'",
        "CREATE INDEX IF NOT EXISTS sentinel_investigations_session ON sentinel_investigations (session_id)",
        // ── diagnoses ────────────────────────────────────────────────────
        r#"CREATE TABLE IF NOT EXISTS sentinel_diagnoses (
  id               TEXT PRIMARY KEY,
  investigation_id TEXT NOT NULL REFERENCES sentinel_investigations(id),
  group_id         TEXT NOT NULL REFERENCES sentinel_groups(id),
  turn_id          TEXT,
  source           TEXT NOT NULL,
  recorded_by      TEXT NOT NULL DEFAULT 'agent',
  diagnosis        TEXT,
  raw_result       TEXT,
  valid            INTEGER NOT NULL DEFAULT 1,
  verification     TEXT,
  created_ms       INTEGER NOT NULL
)"#,
        "CREATE INDEX IF NOT EXISTS sentinel_diagnoses_group ON sentinel_diagnoses (group_id, created_ms DESC)",
        // ── the optional decision tier's tables, empty until it is on ────
        r#"CREATE TABLE IF NOT EXISTS sentinel_decisions (
  id            TEXT PRIMARY KEY,
  point         TEXT NOT NULL,
  subject_kind  TEXT NOT NULL,
  subject_id    TEXT NOT NULL,
  value         TEXT,
  confidence    REAL NOT NULL,
  fell_back     INTEGER NOT NULL DEFAULT 0,
  model         TEXT,
  latency_ms    INTEGER,
  created_ms    INTEGER NOT NULL
)"#,
        "CREATE INDEX IF NOT EXISTS sentinel_decisions_subject ON sentinel_decisions (point, subject_kind, subject_id)",
        // A merge edge never changes a fingerprint: identity stays the hash.
        r#"CREATE TABLE IF NOT EXISTS sentinel_group_edges (
  group_id       TEXT NOT NULL REFERENCES sentinel_groups(id),
  other_group_id TEXT NOT NULL REFERENCES sentinel_groups(id),
  kind           TEXT NOT NULL,
  confidence     REAL NOT NULL,
  status         TEXT NOT NULL,
  decided_by     TEXT,
  created_ms     INTEGER NOT NULL,
  PRIMARY KEY (group_id, other_group_id, kind)
)"#,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_migration_is_the_declared_schema_version() {
        let latest = migrations().last().expect("at least one version").0;
        assert_eq!(latest, SCHEMA_VERSION);
    }

    #[test]
    fn migrations_are_ordered_and_dense() {
        let versions: Vec<u32> = migrations()
            .into_iter()
            .map(|(version, _)| version)
            .collect();
        for (index, version) in versions.iter().enumerate() {
            assert_eq!(
                *version,
                index as u32 + 1,
                "versions start at 1 and never skip"
            );
        }
    }

    #[test]
    fn every_statement_is_reentrant() {
        // Boot runs the pending migrations on every start, including the
        // first: a statement that is not IF NOT EXISTS would fail the second
        // time a version is applied after a partial failure.
        for (version, statements) in migrations() {
            for statement in statements {
                let head = statement
                    .split_whitespace()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" ");
                assert!(
                    head.contains("IF NOT EXISTS") || statement.contains("IF NOT EXISTS"),
                    "v{version} statement is not reentrant: {head}"
                );
            }
        }
    }

    #[test]
    fn a_pending_log_may_have_no_group_and_nothing_else_may() {
        let v1 = migrations();
        let occurrences = v1[0]
            .1
            .iter()
            .find(|statement| statement.contains("sentinel_occurrences ("))
            .expect("the occurrences table");
        assert!(
            occurrences.contains(
                "CHECK ((pending_join = 1 AND group_id IS NULL) OR (pending_join = 0 AND group_id IS NOT NULL))"
            ),
            "the nullable group_id must be bounded by the pending-join state"
        );
    }

    #[test]
    fn only_one_first_pass_can_run_per_group() {
        assert!(migrations()[0].1.iter().any(|statement| statement
            .contains("sentinel_investigations_active")
            && statement.contains("WHERE status = 'running'")));
    }
}
