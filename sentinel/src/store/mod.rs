//! The durable store.
//!
//! Everything the worker knows lives in one `database` connection. Two
//! disciplines are worth stating once, because every write here follows them:
//!
//! **Transactions are one-shot.** The interactive form
//! (`beginTransaction`/…/`commit`) pins a connection across round-trips, and
//! with four ingest jobs in flight that serializes the whole pipeline behind
//! whichever job is slowest. Each write is instead a single
//! `database::transaction` carrying every statement it needs.
//!
//! **State changes are compare-and-set.** A group's transition is decided
//! from a snapshot read a moment earlier, so the `UPDATE` carries the
//! `updated_ms` it saw and returns the id it changed. No row back means
//! somebody moved the group in between — a person resolving it while an
//! occurrence was landing — so the caller re-reads and decides again. That is
//! how "a record never undoes a human decision" survives concurrency instead
//! of being a comment on a function.

pub mod investigations;
pub mod schema;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{
    ids, ErrorSourceV1, GroupCountsV1, GroupState, GroupStatusV1, IgnoreBaselineV1, IgnoreRuleV1,
    SentinelError,
};

/// One statement and its bound parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub sql: String,
    pub params: Vec<Value>,
}

impl Statement {
    pub fn new(sql: impl Into<String>, params: Vec<Value>) -> Self {
        Self {
            sql: sql.into(),
            params,
        }
    }
}

/// A row keyed by column name, as `database::query` returns it.
pub type NamedRow = serde_json::Map<String, Value>;

/// One statement's result inside a transaction. Rows are positional here —
/// that is the shape `database::transaction` reports.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepResult {
    pub affected_rows: u64,
    pub rows: Vec<Vec<Value>>,
}

/// The SQL surface the store needs. Implemented over the `database` worker in
/// production and over an in-process SQLite in tests, so the statements
/// themselves are what gets tested.
#[async_trait]
pub trait Db: Send + Sync {
    async fn query(&self, sql: &str, params: Vec<Value>) -> Result<Vec<NamedRow>, SentinelError>;
    async fn execute(&self, sql: &str, params: Vec<Value>) -> Result<u64, SentinelError>;
    /// Atomic and ordered; rolls back on the first failure.
    async fn transaction(&self, statements: &[Statement])
        -> Result<Vec<StepResult>, SentinelError>;
}

/// What a group looks like to a writer: the identity it is keyed by plus the
/// columns a transition reads.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupRow {
    pub id: String,
    pub updated_ms: i64,
    pub state: GroupState,
}

/// What recording an occurrence did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordOutcome {
    /// A group that did not exist before.
    Created { group_id: String },
    /// An occurrence on a known group, with the status it left it in.
    Counted {
        group_id: String,
        status: GroupStatusV1,
        changed: bool,
    },
    /// The same physical event was already recorded.
    Deduped { group_id: Option<String> },
}

/// A failure ready to be written: identity, attribution and the frozen
/// evidence, all already redacted.
#[derive(Debug, Clone, PartialEq)]
pub struct OccurrenceWrite {
    pub fingerprint: String,
    pub source: ErrorSourceV1,
    pub dedupe_key: String,
    pub at_ms: i64,
    pub namespace: String,
    pub service_name: String,
    pub function_id: Option<String>,
    pub exception_type: Option<String>,
    pub title: String,
    pub message: String,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub worker_version: Option<String>,
    pub evidence: Option<String>,
    pub namespace_ambiguous: bool,
    /// Set when this write promotes a log that was already waiting: the row
    /// exists, so the transaction attaches it to the group instead of
    /// inserting a second one.
    pub pending_occurrence_id: Option<String>,
}

/// Typed access to the store. Generic over [`Db`] so the same statements run
/// against the `database` worker and against a test SQLite.
#[derive(Debug, Clone)]
pub struct Store<D: Db> {
    db: D,
}

/// How many times a compare-and-set is retried before giving up. A miss means
/// another writer moved the row; five rounds is far past any real contention
/// on a single group.
pub(crate) const CAS_ATTEMPTS: usize = 5;

impl<D: Db> Store<D> {
    pub fn new(db: D) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &D {
        &self.db
    }

    /// Create or upgrade the schema. Safe to run on every boot: each pending
    /// version is one atomic batch ending in its own version bump.
    pub async fn migrate(&self) -> Result<u32, SentinelError> {
        self.db.execute(schema::META_TABLE, vec![]).await?;
        let current = self.schema_version().await?;
        let mut applied = current;
        for (version, statements) in schema::migrations() {
            if version <= current {
                continue;
            }
            let mut batch: Vec<Statement> = statements
                .into_iter()
                .map(|sql| Statement::new(sql, vec![]))
                .collect();
            batch.push(Statement::new(
                "INSERT INTO sentinel_meta (key, value) VALUES (?, ?) \
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                vec![
                    json!(schema::SCHEMA_VERSION_KEY),
                    json!(version.to_string()),
                ],
            ));
            self.db.transaction(&batch).await?;
            applied = version;
        }
        Ok(applied)
    }

    pub async fn schema_version(&self) -> Result<u32, SentinelError> {
        let rows = self
            .db
            .query(
                "SELECT value FROM sentinel_meta WHERE key = ?",
                vec![json!(schema::SCHEMA_VERSION_KEY)],
            )
            .await?;
        Ok(rows
            .first()
            .and_then(|row| row.get("value"))
            .and_then(Value::as_str)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0))
    }

    pub async fn group_by_fingerprint(
        &self,
        fingerprint: &str,
    ) -> Result<Option<GroupRow>, SentinelError> {
        let rows = self
            .db
            .query(
                "SELECT id, updated_ms, status, previous_status, occurrence_count, \
                 resolved_version, resolve_until_version_change, ignore_rule, ignore_baseline, \
                 diagnosis_id FROM sentinel_groups WHERE fingerprint = ?",
                vec![json!(fingerprint)],
            )
            .await?;
        rows.first().map(group_row).transpose()
    }

    pub async fn group_by_id(&self, group_id: &str) -> Result<Option<GroupRow>, SentinelError> {
        let rows = self
            .db
            .query(
                "SELECT id, updated_ms, status, previous_status, occurrence_count, \
                 resolved_version, resolve_until_version_change, ignore_rule, ignore_baseline, \
                 diagnosis_id FROM sentinel_groups WHERE id = ?",
                vec![json!(group_id)],
            )
            .await?;
        rows.first().map(group_row).transpose()
    }

    /// Whether this exact physical event was already recorded.
    pub async fn occurrence_group(
        &self,
        dedupe_key: &str,
    ) -> Result<Option<Option<String>>, SentinelError> {
        let rows = self
            .db
            .query(
                "SELECT group_id FROM sentinel_occurrences WHERE dedupe_key = ?",
                vec![json!(dedupe_key)],
            )
            .await?;
        Ok(rows.first().map(|row| {
            row.get("group_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        }))
    }

    /// Record one occurrence, creating its group when it is the first.
    ///
    /// The state change is decided by [`crate::lifecycle`] from the row read
    /// here and written under compare-and-set in the same transaction, so a
    /// person resolving the group mid-flight wins rather than being silently
    /// reverted.
    pub async fn record_occurrence(
        &self,
        write: &OccurrenceWrite,
    ) -> Result<RecordOutcome, SentinelError> {
        // A promotion is not a duplicate of itself: the row it attaches was
        // inserted under this very key when the log arrived.
        if write.pending_occurrence_id.is_none() {
            if let Some(group_id) = self.occurrence_group(&write.dedupe_key).await? {
                return Ok(RecordOutcome::Deduped { group_id });
            }
        }

        for _ in 0..CAS_ATTEMPTS {
            let Some(existing) = self.group_by_fingerprint(&write.fingerprint).await? else {
                match self.create_group(write).await {
                    Ok(group_id) => return Ok(RecordOutcome::Created { group_id }),
                    // Another writer created the same fingerprint first;
                    // fall through and count against theirs.
                    Err(SentinelError::Dependency(_)) => continue,
                    Err(error) => return Err(error),
                }
            };

            let transition =
                crate::lifecycle::on_occurrence(&existing.state, write.worker_version.as_deref());
            let statements = self.count_statements(write, &existing, &transition);
            let results = self.db.transaction(&statements).await?;
            let changed_group = results
                .first()
                .is_some_and(|step| step.affected_rows > 0 || !step.rows.is_empty());
            if !changed_group {
                // Someone moved the group between the read and the write.
                continue;
            }
            return Ok(RecordOutcome::Counted {
                group_id: existing.id,
                status: transition.status,
                changed: transition.moved(existing.state.status) || transition.reason.is_some(),
            });
        }
        Err(SentinelError::dependency(
            "the group changed under every attempt to record an occurrence",
        ))
    }

    async fn create_group(&self, write: &OccurrenceWrite) -> Result<String, SentinelError> {
        let group_id = ids::group_id();
        let occurrence_id = ids::occurrence_id();
        let bucket = ids::hour_bucket_ms(write.at_ms);
        let mut statements = vec![
            Statement::new(
                "INSERT INTO sentinel_groups (id, fingerprint, source, namespace, service_name, \
                 function_id, exception_type, title, message_sample, status, first_seen_ms, \
                 last_seen_ms, occurrence_count, first_version, last_version, updated_ms) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'new', ?, ?, 1, ?, ?, ?)",
                vec![
                    json!(group_id),
                    json!(write.fingerprint),
                    json!(write.source.as_str()),
                    json!(write.namespace),
                    json!(write.service_name),
                    json!(write.function_id),
                    json!(write.exception_type),
                    json!(write.title),
                    json!(write.message),
                    json!(write.at_ms),
                    json!(write.at_ms),
                    json!(write.worker_version),
                    json!(write.worker_version),
                    json!(write.at_ms),
                ],
            ),
            self.insert_occurrence(&occurrence_id, &group_id, write),
            Statement::new(
                "INSERT INTO sentinel_buckets (group_id, hour_ms, count) VALUES (?, ?, 1) \
                 ON CONFLICT (group_id, hour_ms) DO UPDATE SET count = count + 1",
                vec![json!(group_id), json!(bucket)],
            ),
        ];
        statements.extend(self.session_statement(&group_id, write));
        statements.extend(self.fold_pending_statement(write));
        self.db.transaction(&statements).await?;
        Ok(group_id)
    }

    fn count_statements(
        &self,
        write: &OccurrenceWrite,
        existing: &GroupRow,
        transition: &crate::Transition,
    ) -> Vec<Statement> {
        let occurrence_id = ids::occurrence_id();
        let bucket = ids::hour_bucket_ms(write.at_ms);
        let moved = transition.moved(existing.state.status);
        let mut statements = vec![Statement::new(
            "UPDATE sentinel_groups SET occurrence_count = occurrence_count + 1, \
             last_seen_ms = MAX(last_seen_ms, ?), last_version = COALESCE(?, last_version), \
             status = ?, previous_status = CASE WHEN ? THEN status ELSE previous_status END, \
             regressed_at_ms = CASE WHEN ? THEN ? ELSE regressed_at_ms END, \
             ignore_rule = CASE WHEN ? THEN NULL ELSE ignore_rule END, \
             ignore_baseline = CASE WHEN ? THEN NULL ELSE ignore_baseline END, \
             updated_ms = ? WHERE id = ? AND updated_ms = ? RETURNING id",
            vec![
                json!(write.at_ms),
                json!(write.worker_version),
                json!(transition.status.as_str()),
                json!(moved),
                json!(transition.status == GroupStatusV1::Regressed),
                json!(write.at_ms),
                json!(transition.clear_ignore),
                json!(transition.clear_ignore),
                json!(write.at_ms),
                json!(existing.id),
                json!(existing.updated_ms),
            ],
        )];
        statements.push(self.insert_occurrence(&occurrence_id, &existing.id, write));
        statements.push(Statement::new(
            "INSERT INTO sentinel_buckets (group_id, hour_ms, count) VALUES (?, ?, 1) \
             ON CONFLICT (group_id, hour_ms) DO UPDATE SET count = count + 1",
            vec![json!(existing.id), json!(bucket)],
        ));
        statements.extend(self.session_statement(&existing.id, write));
        statements.extend(self.fold_pending_statement(write));
        statements
    }

    fn insert_occurrence(
        &self,
        occurrence_id: &str,
        group_id: &str,
        write: &OccurrenceWrite,
    ) -> Statement {
        // Promotion: the row has been waiting for its span since the log
        // arrived. Attaching it and clearing the wait is one statement, so
        // the row can never be visible as settled-but-groupless.
        if let Some(pending_id) = &write.pending_occurrence_id {
            return Statement::new(
                "UPDATE sentinel_occurrences SET group_id = ?, pending_join = 0, \
                 join_deadline_ms = NULL, message = ?, evidence = ?, evidence_bytes = ? \
                 WHERE id = ? AND pending_join = 1",
                vec![
                    json!(group_id),
                    json!(write.message),
                    json!(write.evidence),
                    json!(write.evidence.as_ref().map(String::len).unwrap_or(0)),
                    json!(pending_id),
                ],
            );
        }
        Statement::new(
            "INSERT INTO sentinel_occurrences (id, group_id, dedupe_key, source, at_ms, trace_id, \
             span_id, session_id, turn_id, worker_version, message, evidence, evidence_bytes, \
             namespace_ambiguous) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (dedupe_key) DO NOTHING",
            vec![
                json!(occurrence_id),
                json!(group_id),
                json!(write.dedupe_key),
                json!(write.source.as_str()),
                json!(write.at_ms),
                json!(write.trace_id),
                json!(write.span_id),
                json!(write.session_id),
                json!(write.turn_id),
                json!(write.worker_version),
                json!(write.message),
                json!(write.evidence),
                json!(write.evidence.as_ref().map(String::len).unwrap_or(0)),
                json!(write.namespace_ambiguous),
            ],
        )
    }

    /// One row per distinct session, never pruned with the occurrences — it
    /// is what keeps "sessions affected" true once the row cap starts cutting.
    fn session_statement(&self, group_id: &str, write: &OccurrenceWrite) -> Option<Statement> {
        let session_id = write.session_id.as_ref()?;
        Some(Statement::new(
            "INSERT INTO sentinel_group_sessions (group_id, session_id, first_ms, last_ms) \
             VALUES (?, ?, ?, ?) ON CONFLICT (group_id, session_id) \
             DO UPDATE SET last_ms = MAX(last_ms, excluded.last_ms)",
            vec![
                json!(group_id),
                json!(session_id),
                json!(write.at_ms),
                json!(write.at_ms),
            ],
        ))
    }

    /// A span arriving for a trace that has ERROR logs waiting: the logs
    /// belong to this occurrence's evidence, so their placeholder rows go.
    fn fold_pending_statement(&self, write: &OccurrenceWrite) -> Option<Statement> {
        if write.source != ErrorSourceV1::Trace {
            return None;
        }
        let trace_id = write.trace_id.as_ref()?;
        Some(Statement::new(
            "DELETE FROM sentinel_occurrences WHERE pending_join = 1 AND trace_id = ?",
            vec![json!(trace_id)],
        ))
    }

    /// Park an ERROR log until the span that explains it arrives.
    pub async fn insert_pending_log(
        &self,
        pending: &PendingLogWrite,
    ) -> Result<Option<String>, SentinelError> {
        if self.occurrence_group(&pending.dedupe_key).await?.is_some() {
            return Ok(None);
        }
        let occurrence_id = ids::occurrence_id();
        self.db
            .transaction(&[Statement::new(
                "INSERT INTO sentinel_occurrences (id, group_id, dedupe_key, source, at_ms, \
                 trace_id, span_id, session_id, worker_version, message, evidence, \
                 evidence_bytes, pending_join, join_deadline_ms, session_unknown) \
                 VALUES (?, NULL, ?, 'log', ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?) \
                 ON CONFLICT (dedupe_key) DO NOTHING",
                vec![
                    json!(occurrence_id),
                    json!(pending.dedupe_key),
                    json!(pending.at_ms),
                    json!(pending.trace_id),
                    json!(pending.span_id),
                    json!(pending.session_id),
                    json!(pending.worker_version),
                    json!(pending.message),
                    json!(pending.evidence),
                    json!(pending.evidence.as_ref().map(String::len).unwrap_or(0)),
                    json!(pending.join_deadline_ms),
                    json!(pending.session_unknown),
                ],
            )])
            .await?;
        Ok(Some(occurrence_id))
    }

    /// Logs whose wait has run out, oldest first.
    pub async fn due_pending_logs(
        &self,
        now_ms: i64,
        limit: usize,
    ) -> Result<Vec<PendingLog>, SentinelError> {
        let rows = self
            .db
            .query(
                "SELECT id, dedupe_key, at_ms, trace_id, span_id, session_id, worker_version, \
                 message, evidence, session_unknown FROM sentinel_occurrences \
                 WHERE pending_join = 1 AND join_deadline_ms <= ? ORDER BY join_deadline_ms LIMIT ?",
                vec![json!(now_ms), json!(limit as i64)],
            )
            .await?;
        Ok(rows.iter().map(pending_log).collect())
    }

    /// How many logs are holding right now — a gauge for `sentinel::status`.
    pub async fn pending_log_count(&self) -> Result<u64, SentinelError> {
        let rows = self
            .db
            .query(
                "SELECT COUNT(*) AS total FROM sentinel_occurrences WHERE pending_join = 1",
                vec![],
            )
            .await?;
        Ok(rows
            .first()
            .and_then(|row| row.get("total"))
            .and_then(Value::as_i64)
            .unwrap_or(0) as u64)
    }

    pub async fn pending_log(
        &self,
        occurrence_id: &str,
    ) -> Result<Option<PendingLog>, SentinelError> {
        let rows = self
            .db
            .query(
                "SELECT id, dedupe_key, at_ms, trace_id, span_id, session_id, worker_version, \
                 message, evidence, session_unknown FROM sentinel_occurrences \
                 WHERE id = ? AND pending_join = 1",
                vec![json!(occurrence_id)],
            )
            .await?;
        Ok(rows.first().map(pending_log))
    }

    /// The span-sourced occurrence of a trace, when one is already recorded:
    /// a log arriving after its span folds into this rather than waiting.
    pub async fn trace_occurrence(
        &self,
        trace_id: &str,
    ) -> Result<Option<(String, String, Option<String>)>, SentinelError> {
        let rows = self
            .db
            .query(
                "SELECT id, group_id, evidence FROM sentinel_occurrences \
                 WHERE trace_id = ? AND source = 'trace' AND pending_join = 0 \
                 ORDER BY at_ms DESC LIMIT 1",
                vec![json!(trace_id)],
            )
            .await?;
        Ok(rows.first().and_then(|row| {
            Some((
                text(row, "id")?,
                text(row, "group_id")?,
                text(row, "evidence"),
            ))
        }))
    }

    /// Replace one occurrence's frozen evidence — the settle pass, and a log
    /// folding into the span that explains it.
    pub async fn replace_evidence(
        &self,
        occurrence_id: &str,
        evidence: &str,
        settled: bool,
    ) -> Result<(), SentinelError> {
        self.db
            .execute(
                "UPDATE sentinel_occurrences SET evidence = ?, evidence_bytes = ?, \
                 settled = MAX(settled, ?) WHERE id = ?",
                vec![
                    json!(evidence),
                    json!(evidence.len()),
                    json!(settled),
                    json!(occurrence_id),
                ],
            )
            .await?;
        Ok(())
    }

    /// Group counts for `sentinel::status`, in one pass.
    pub async fn group_counts(&self) -> Result<GroupCountsV1, SentinelError> {
        let rows = self
            .db
            .query(
                "SELECT status, COUNT(*) AS total FROM sentinel_groups \
                 WHERE archived = 0 GROUP BY status",
                vec![],
            )
            .await?;
        let mut counts = GroupCountsV1::default();
        for row in rows {
            let total = row.get("total").and_then(Value::as_i64).unwrap_or(0) as u64;
            match row.get("status").and_then(Value::as_str) {
                Some("regressed") => {
                    counts.regressed += total;
                    counts.open += total;
                }
                Some("ignored") => counts.ignored += total,
                Some("resolved") => counts.resolved += total,
                Some("new" | "investigating" | "diagnosed") => counts.open += total,
                _ => {}
            }
        }
        Ok(counts)
    }
}

/// An ERROR log waiting for the span of its own trace.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingLogWrite {
    pub dedupe_key: String,
    pub at_ms: i64,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub session_id: Option<String>,
    pub worker_version: Option<String>,
    pub message: String,
    pub evidence: Option<String>,
    pub join_deadline_ms: i64,
    /// The trace could not be resolved, so the session — and the exclusion
    /// checks that depend on it — are unknown. Bounded and visible, never
    /// silent.
    pub session_unknown: bool,
}

/// A parked log, read back when its wait ends.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingLog {
    pub id: String,
    pub dedupe_key: String,
    pub at_ms: i64,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub session_id: Option<String>,
    pub worker_version: Option<String>,
    pub message: String,
    pub evidence: Option<String>,
    pub session_unknown: bool,
}

fn pending_log(row: &NamedRow) -> PendingLog {
    PendingLog {
        id: text(row, "id").unwrap_or_default(),
        dedupe_key: text(row, "dedupe_key").unwrap_or_default(),
        at_ms: number(row, "at_ms").unwrap_or_default(),
        trace_id: text(row, "trace_id"),
        span_id: text(row, "span_id"),
        session_id: text(row, "session_id"),
        worker_version: text(row, "worker_version"),
        message: text(row, "message").unwrap_or_default(),
        evidence: text(row, "evidence"),
        session_unknown: flag(row, "session_unknown"),
    }
}

fn group_row(row: &NamedRow) -> Result<GroupRow, SentinelError> {
    let id = text(row, "id").ok_or_else(|| malformed("id"))?;
    let status = text(row, "status")
        .and_then(|value| status_from_str(&value))
        .ok_or_else(|| malformed("status"))?;
    Ok(GroupRow {
        id,
        updated_ms: number(row, "updated_ms").unwrap_or_default(),
        state: GroupState {
            status,
            occurrence_count: number(row, "occurrence_count").unwrap_or_default() as u64,
            previous_status: text(row, "previous_status").and_then(|v| status_from_str(&v)),
            resolved_version: text(row, "resolved_version"),
            resolve_until_version_change: flag(row, "resolve_until_version_change"),
            ignore_rule: text(row, "ignore_rule")
                .and_then(|value| serde_json::from_str::<IgnoreRuleV1>(&value).ok()),
            ignore_baseline: text(row, "ignore_baseline")
                .and_then(|value| serde_json::from_str::<IgnoreBaselineV1>(&value).ok()),
            has_diagnosis: text(row, "diagnosis_id").is_some(),
        },
    })
}

fn status_from_str(value: &str) -> Option<GroupStatusV1> {
    match value {
        "new" => Some(GroupStatusV1::New),
        "investigating" => Some(GroupStatusV1::Investigating),
        "diagnosed" => Some(GroupStatusV1::Diagnosed),
        "resolved" => Some(GroupStatusV1::Resolved),
        "regressed" => Some(GroupStatusV1::Regressed),
        "ignored" => Some(GroupStatusV1::Ignored),
        _ => None,
    }
}

fn text(row: &NamedRow, column: &str) -> Option<String> {
    row.get(column)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

fn number(row: &NamedRow, column: &str) -> Option<i64> {
    row.get(column).and_then(Value::as_i64)
}

/// SQLite has no boolean: an integer column reads back as 0 or 1, and a
/// driver that does map it to JSON `true` is handled too.
fn flag(row: &NamedRow, column: &str) -> bool {
    match row.get(column) {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_i64().unwrap_or(0) != 0,
        _ => false,
    }
}

fn malformed(column: &str) -> SentinelError {
    SentinelError::dependency(format!("group row is missing `{column}`"))
}
