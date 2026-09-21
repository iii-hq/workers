//! The read and mutate surface: what the console page calls.
//!
//! Two things here are deliberate. Listing sorts regressions first, because a
//! failure that came back after somebody fixed it is the one piece of news in
//! the list. And every mutation goes through the same compare-and-set the
//! ingest uses, so "only a human resolves" holds even when an occurrence
//! lands in the same instant.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::store::{Db, Statement, Store};
use crate::{
    evidence::EvidenceBundleV1, ids, lifecycle, ErrorSourceV1, EvidenceGetRequestV1,
    EvidenceGetResponseV1, GroupActionRequestV1, GroupChangeReasonV1, GroupGetRequestV1,
    GroupGetResponseV1, GroupStateResponseV1, GroupStatusV1, GroupSummaryV1, GroupsListRequestV1,
    GroupsListResponseV1, IgnoreBaselineV1, IgnoreRequestV1, IgnoreRuleV1, NamedRow,
    OccurrenceSummaryV1, OccurrencesListRequestV1, OccurrencesListResponseV1, ResolveRequestV1,
    SentinelError, Transition,
};

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;
const SPARKLINE_HOURS: i64 = 24;
const HOUR_MS: i64 = 3_600_000;

/// Whether the trace behind an occurrence is still readable in the engine.
#[async_trait::async_trait]
pub trait TraceAvailability: Send + Sync {
    async fn trace_exists(&self, trace_id: &str) -> bool;
}

pub struct Service<D: Db> {
    store: Arc<Store<D>>,
    traces: Arc<dyn TraceAvailability>,
}

impl<D: Db> Service<D> {
    pub fn new(store: Arc<Store<D>>, traces: Arc<dyn TraceAvailability>) -> Self {
        Self { store, traces }
    }

    pub async fn list(
        &self,
        request: GroupsListRequestV1,
    ) -> Result<GroupsListResponseV1, SentinelError> {
        let statuses = request.status.clone().unwrap_or_else(open_states);
        let limit = request.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
        let offset = request.offset.unwrap_or(0);

        let mut where_sql = String::from("archived = 0");
        let mut params: Vec<Value> = Vec::new();
        if !statuses.is_empty() {
            let slots = vec!["?"; statuses.len()].join(", ");
            where_sql.push_str(&format!(" AND status IN ({slots})"));
            params.extend(statuses.iter().map(|status| json!(status.as_str())));
        }
        if let Some(service_name) = request.service_name.as_deref().filter(|s| !s.is_empty()) {
            where_sql.push_str(" AND service_name = ?");
            params.push(json!(service_name));
        }
        if let Some(since_ms) = request.since_ms {
            where_sql.push_str(" AND last_seen_ms >= ?");
            params.push(json!(since_ms));
        }
        if let Some(search) = request
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            where_sql
                .push_str(" AND (title LIKE ? OR message_sample LIKE ? OR function_id LIKE ?)");
            let pattern = format!("%{}%", escape_like(search));
            params.extend([
                json!(pattern.clone()),
                json!(pattern.clone()),
                json!(pattern),
            ]);
        }

        let total = self
            .store
            .db()
            .query(
                &format!("SELECT COUNT(*) AS total FROM sentinel_groups WHERE {where_sql}"),
                params.clone(),
            )
            .await?
            .first()
            .and_then(|row| row.get("total"))
            .and_then(Value::as_i64)
            .unwrap_or(0) as u64;

        let mut page_params = params;
        page_params.push(json!(limit as i64));
        page_params.push(json!(offset as i64));
        let rows = self
            .store
            .db()
            .query(
                &format!(
                    "SELECT * FROM sentinel_groups WHERE {where_sql} \
                     ORDER BY (status = 'regressed') DESC, \
                     COALESCE(regressed_at_ms, 0) DESC, last_seen_ms DESC \
                     LIMIT ? OFFSET ?"
                ),
                page_params,
            )
            .await?;

        let mut groups = Vec::with_capacity(rows.len());
        for row in &rows {
            groups.push(self.summary(row).await?);
        }
        Ok(GroupsListResponseV1 { groups, total })
    }

    pub async fn get(
        &self,
        request: GroupGetRequestV1,
    ) -> Result<GroupGetResponseV1, SentinelError> {
        let row = self.group_row(&request.group_id).await?;
        let group = self.summary(&row).await?;
        let latest = self.latest_occurrence(&request.group_id).await?;
        let trace_available = match latest.as_ref().and_then(|o| o.trace_id.as_deref()) {
            Some(trace_id) => self.traces.trace_exists(trace_id).await,
            None => false,
        };
        Ok(GroupGetResponseV1 {
            group,
            message_sample: text(&row, "message_sample").unwrap_or_default(),
            latest_occurrence: latest,
            trace_available,
        })
    }

    pub async fn occurrences(
        &self,
        request: OccurrencesListRequestV1,
    ) -> Result<OccurrencesListResponseV1, SentinelError> {
        let limit = request.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
        let offset = request.offset.unwrap_or(0);
        let total = self
            .store
            .db()
            .query(
                "SELECT COUNT(*) AS total FROM sentinel_occurrences WHERE group_id = ?",
                vec![json!(request.group_id)],
            )
            .await?
            .first()
            .and_then(|row| row.get("total"))
            .and_then(Value::as_i64)
            .unwrap_or(0) as u64;
        let rows = self
            .store
            .db()
            .query(
                "SELECT * FROM sentinel_occurrences WHERE group_id = ? \
                 ORDER BY at_ms DESC LIMIT ? OFFSET ?",
                vec![
                    json!(request.group_id),
                    json!(limit as i64),
                    json!(offset as i64),
                ],
            )
            .await?;
        Ok(OccurrencesListResponseV1 {
            occurrences: rows.iter().map(occurrence).collect(),
            total,
        })
    }

    pub async fn evidence(
        &self,
        request: EvidenceGetRequestV1,
    ) -> Result<EvidenceGetResponseV1, SentinelError> {
        let rows = self
            .store
            .db()
            .query(
                "SELECT evidence FROM sentinel_occurrences WHERE id = ?",
                vec![json!(request.occurrence_id)],
            )
            .await?;
        let row = rows.first().ok_or_else(|| {
            SentinelError::NotFound(format!("occurrence {}", request.occurrence_id))
        })?;
        let evidence: Option<EvidenceBundleV1> =
            text(row, "evidence").and_then(|json| serde_json::from_str(&json).ok());
        Ok(EvidenceGetResponseV1 {
            pruned: evidence.is_none(),
            evidence,
        })
    }

    pub async fn resolve(
        &self,
        request: ResolveRequestV1,
    ) -> Result<GroupStateResponseV1, SentinelError> {
        self.mutate(&request.group_id, lifecycle::resolve, |row, transition| {
            let version = text(row, "last_version");
            (
                "status = ?, previous_status = status, resolved_at_ms = ?, resolved_version = ?, \
                 resolve_until_version_change = ?, regressed_at_ms = NULL, ignore_rule = NULL, \
                 ignore_baseline = NULL, updated_ms = ?"
                    .to_string(),
                vec![
                    json!(transition.status.as_str()),
                    json!(ids::now_ms()),
                    json!(version),
                    json!(request.until_version_change),
                    json!(ids::now_ms()),
                ],
            )
        })
        .await
    }

    pub async fn ignore(
        &self,
        request: IgnoreRequestV1,
    ) -> Result<GroupStateResponseV1, SentinelError> {
        self.mutate(&request.group_id, lifecycle::ignore, |row, transition| {
            // The baseline is captured now: "50 more occurrences" counts from
            // the moment somebody said it, not from the group's first one.
            let baseline = IgnoreBaselineV1 {
                occurrence_count: number(row, "occurrence_count").unwrap_or_default() as u64,
                worker_version: text(row, "last_version"),
            };
            (
                "status = ?, previous_status = status, ignore_rule = ?, ignore_baseline = ?, \
                 updated_ms = ?"
                    .to_string(),
                vec![
                    json!(transition.status.as_str()),
                    json!(serde_json::to_string(&request.rule).unwrap_or_default()),
                    json!(serde_json::to_string(&baseline).unwrap_or_default()),
                    json!(ids::now_ms()),
                ],
            )
        })
        .await
    }

    pub async fn unignore(
        &self,
        request: GroupActionRequestV1,
    ) -> Result<GroupStateResponseV1, SentinelError> {
        self.reopen_like(&request.group_id, lifecycle::unignore)
            .await
    }

    pub async fn reopen(
        &self,
        request: GroupActionRequestV1,
    ) -> Result<GroupStateResponseV1, SentinelError> {
        self.reopen_like(&request.group_id, lifecycle::reopen).await
    }

    async fn reopen_like(
        &self,
        group_id: &str,
        decide: fn(&crate::GroupState) -> Result<Transition, SentinelError>,
    ) -> Result<GroupStateResponseV1, SentinelError> {
        self.mutate(group_id, decide, |_row, transition| {
            (
                "status = ?, previous_status = status, ignore_rule = NULL, \
                 ignore_baseline = NULL, resolved_at_ms = NULL, resolved_version = NULL, \
                 resolve_until_version_change = 0, regressed_at_ms = NULL, updated_ms = ?"
                    .to_string(),
                vec![json!(transition.status.as_str()), json!(ids::now_ms())],
            )
        })
        .await
    }

    /// Read, decide, write under compare-and-set, retrying when the row moved.
    async fn mutate<F, S>(
        &self,
        group_id: &str,
        decide: F,
        statement: S,
    ) -> Result<GroupStateResponseV1, SentinelError>
    where
        F: Fn(&crate::GroupState) -> Result<Transition, SentinelError>,
        S: Fn(&NamedRow, &Transition) -> (String, Vec<Value>),
    {
        for _ in 0..5 {
            let row = self.group_row(group_id).await?;
            let current = self
                .store
                .group_by_id(group_id)
                .await?
                .ok_or_else(|| SentinelError::NotFound(format!("group {group_id}")))?;
            let transition = decide(&current.state)?;
            if transition.reason.is_none() && transition.status == current.state.status {
                // Nothing to do: the decision was already taken.
                return Ok(GroupStateResponseV1 {
                    group_id: group_id.to_string(),
                    status: current.state.status,
                    reason: None,
                });
            }
            let (set_sql, mut params) = statement(&row, &transition);
            params.push(json!(group_id));
            params.push(json!(current.updated_ms));
            let results = self
                .store
                .db()
                .transaction(&[Statement::new(
                    format!(
                        "UPDATE sentinel_groups SET {set_sql} WHERE id = ? AND updated_ms = ? \
                         RETURNING id"
                    ),
                    params,
                )])
                .await?;
            let applied = results
                .first()
                .is_some_and(|step| step.affected_rows > 0 || !step.rows.is_empty());
            if applied {
                return Ok(GroupStateResponseV1 {
                    group_id: group_id.to_string(),
                    status: transition.status,
                    reason: transition.reason,
                });
            }
        }
        Err(SentinelError::dependency(
            "the group changed under every attempt to apply the decision",
        ))
    }

    async fn group_row(&self, group_id: &str) -> Result<NamedRow, SentinelError> {
        self.store
            .db()
            .query(
                "SELECT * FROM sentinel_groups WHERE id = ?",
                vec![json!(group_id)],
            )
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| SentinelError::NotFound(format!("group {group_id}")))
    }

    async fn summary(&self, row: &NamedRow) -> Result<GroupSummaryV1, SentinelError> {
        let id = text(row, "id").unwrap_or_default();
        Ok(GroupSummaryV1 {
            sessions_affected: self.sessions_affected(&id).await?,
            sparkline: self.sparkline(&id).await?,
            id,
            fingerprint: text(row, "fingerprint").unwrap_or_default(),
            source: source_of(text(row, "source").as_deref()),
            namespace: text(row, "namespace").unwrap_or_default(),
            service_name: text(row, "service_name").unwrap_or_default(),
            function_id: text(row, "function_id"),
            exception_type: text(row, "exception_type"),
            title: text(row, "title").unwrap_or_default(),
            status: status_of(text(row, "status").as_deref()),
            occurrence_count: number(row, "occurrence_count").unwrap_or_default() as u64,
            first_seen_ms: number(row, "first_seen_ms").unwrap_or_default(),
            last_seen_ms: number(row, "last_seen_ms").unwrap_or_default(),
            first_version: text(row, "first_version"),
            last_version: text(row, "last_version"),
            has_diagnosis: text(row, "diagnosis_id").is_some(),
            ignore_rule: text(row, "ignore_rule")
                .and_then(|json| serde_json::from_str::<IgnoreRuleV1>(&json).ok()),
            regressed_at_ms: number(row, "regressed_at_ms"),
        })
    }

    async fn sessions_affected(&self, group_id: &str) -> Result<u64, SentinelError> {
        Ok(self
            .store
            .db()
            .query(
                "SELECT COUNT(*) AS total FROM sentinel_group_sessions WHERE group_id = ?",
                vec![json!(group_id)],
            )
            .await?
            .first()
            .and_then(|row| row.get("total"))
            .and_then(Value::as_i64)
            .unwrap_or(0) as u64)
    }

    /// The last day as hourly counts, oldest first — read from the buckets
    /// rather than by scanning occurrences.
    async fn sparkline(&self, group_id: &str) -> Result<Vec<u64>, SentinelError> {
        let now_hour = ids::hour_bucket_ms(ids::now_ms());
        let from = now_hour - (SPARKLINE_HOURS - 1) * HOUR_MS;
        let rows = self
            .store
            .db()
            .query(
                "SELECT hour_ms, count FROM sentinel_buckets WHERE group_id = ? AND hour_ms >= ?",
                vec![json!(group_id), json!(from)],
            )
            .await?;
        let mut bars = vec![0u64; SPARKLINE_HOURS as usize];
        for row in rows {
            let Some(hour) = row.get("hour_ms").and_then(Value::as_i64) else {
                continue;
            };
            let index = ((hour - from) / HOUR_MS) as usize;
            if index < bars.len() {
                bars[index] = row.get("count").and_then(Value::as_i64).unwrap_or(0) as u64;
            }
        }
        Ok(bars)
    }

    async fn latest_occurrence(
        &self,
        group_id: &str,
    ) -> Result<Option<OccurrenceSummaryV1>, SentinelError> {
        Ok(self
            .store
            .db()
            .query(
                "SELECT * FROM sentinel_occurrences WHERE group_id = ? ORDER BY at_ms DESC LIMIT 1",
                vec![json!(group_id)],
            )
            .await?
            .first()
            .map(occurrence))
    }
}

fn occurrence(row: &NamedRow) -> OccurrenceSummaryV1 {
    OccurrenceSummaryV1 {
        id: text(row, "id").unwrap_or_default(),
        at_ms: number(row, "at_ms").unwrap_or_default(),
        source: source_of(text(row, "source").as_deref()),
        trace_id: text(row, "trace_id"),
        span_id: text(row, "span_id"),
        session_id: text(row, "session_id"),
        turn_id: text(row, "turn_id"),
        worker_version: text(row, "worker_version"),
        message: text(row, "message").unwrap_or_default(),
        has_evidence: text(row, "evidence").is_some(),
        settled: flag(row, "settled"),
        namespace_ambiguous: flag(row, "namespace_ambiguous"),
    }
}

fn open_states() -> Vec<GroupStatusV1> {
    vec![
        GroupStatusV1::New,
        GroupStatusV1::Investigating,
        GroupStatusV1::Diagnosed,
        GroupStatusV1::Regressed,
    ]
}

/// `LIKE` treats these as wildcards; a search for `100%` should find `100%`.
fn escape_like(value: &str) -> String {
    value.replace(['%', '_'], "")
}

fn status_of(value: Option<&str>) -> GroupStatusV1 {
    match value {
        Some("investigating") => GroupStatusV1::Investigating,
        Some("diagnosed") => GroupStatusV1::Diagnosed,
        Some("resolved") => GroupStatusV1::Resolved,
        Some("regressed") => GroupStatusV1::Regressed,
        Some("ignored") => GroupStatusV1::Ignored,
        _ => GroupStatusV1::New,
    }
}

fn source_of(value: Option<&str>) -> ErrorSourceV1 {
    match value {
        Some("log") => ErrorSourceV1::Log,
        Some("harness-turn") => ErrorSourceV1::HarnessTurn,
        Some("report") => ErrorSourceV1::Report,
        _ => ErrorSourceV1::Trace,
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

fn flag(row: &NamedRow, column: &str) -> bool {
    match row.get(column) {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_i64().unwrap_or(0) != 0,
        _ => false,
    }
}

/// Reasons a mutation reports, for the event the console listens to.
pub fn reason_of(response: &GroupStateResponseV1) -> Option<GroupChangeReasonV1> {
    response.reason
}
