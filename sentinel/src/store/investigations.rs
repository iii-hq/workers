//! Investigations and the diagnoses they record.
//!
//! Two invariants live here rather than in the code above. **One running
//! first pass per group** is the partial unique index in the schema, so a
//! second `investigate` racing the first loses at the database instead of
//! creating a duplicate session. And **a diagnosis and the group's move to
//! `diagnosed` are one transaction**, under the same compare-and-set as every
//! other transition — which is what makes "a person clicking Resolve beats a
//! diagnosis still in flight" true rather than merely intended.

use serde_json::{json, Value};

use super::{number, text, Db, Statement, Store};
use crate::{
    ids, lifecycle, DiagnosisRecordV1, DiagnosisSourceV1, DiagnosisV1, GroupStatusV1,
    InvestigationCountsV1, InvestigationModeV1, InvestigationStatusV1, InvestigationSummaryV1,
    NamedRow, SentinelError,
};

/// A new investigation, before the harness has been told anything.
#[derive(Debug, Clone, PartialEq)]
pub struct InvestigationWrite {
    pub id: String,
    pub group_id: String,
    pub occurrence_id: String,
    pub session_id: String,
    pub mode: InvestigationModeV1,
    pub model: String,
    pub provider: Option<String>,
    pub repository_id: Option<String>,
    pub checkout_ref: Option<String>,
    pub investigated_version: Option<String>,
    pub status: InvestigationStatusV1,
    pub requested_by: Option<String>,
    pub created_ms: i64,
}

/// What a recorded diagnosis left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosisOutcome {
    pub diagnosis_id: String,
    pub version: u64,
    pub group_status: GroupStatusV1,
    /// Whether the group's own state moved, and so whether the console needs
    /// to hear about it.
    pub group_moved: bool,
}

impl<D: Db> Store<D> {
    /// Open an investigation. The partial unique index refuses a second
    /// running one for the same group, which is reported as `Ok(false)` so
    /// the caller can hand back the existing session.
    pub async fn insert_investigation(
        &self,
        write: &InvestigationWrite,
    ) -> Result<bool, SentinelError> {
        let result = self
            .db()
            .execute(
                "INSERT INTO sentinel_investigations \
                 (id, group_id, occurrence_id, session_id, mode, nudged, model, provider, \
                  repository_id, checkout_ref, investigated_version, status, requested_by, \
                  created_ms) \
                 VALUES (?, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?, ?, ?, ?)",
                vec![
                    json!(write.id),
                    json!(write.group_id),
                    json!(write.occurrence_id),
                    json!(write.session_id),
                    json!(write.mode.as_str()),
                    json!(write.model),
                    json!(write.provider),
                    json!(write.repository_id),
                    json!(write.checkout_ref),
                    json!(write.investigated_version),
                    json!(write.status.as_str()),
                    json!(write.requested_by),
                    json!(write.created_ms),
                ],
            )
            .await;
        match result {
            Ok(_) => Ok(true),
            Err(error) if is_conflict(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub async fn investigation_by_id(
        &self,
        investigation_id: &str,
    ) -> Result<Option<InvestigationSummaryV1>, SentinelError> {
        Ok(self
            .db()
            .query(
                "SELECT * FROM sentinel_investigations WHERE id = ?",
                vec![json!(investigation_id)],
            )
            .await?
            .first()
            .map(investigation))
    }

    /// The first pass in flight for a group, if there is one.
    pub async fn running_investigation(
        &self,
        group_id: &str,
    ) -> Result<Option<InvestigationSummaryV1>, SentinelError> {
        Ok(self
            .db()
            .query(
                "SELECT * FROM sentinel_investigations WHERE group_id = ? AND status = 'running' \
                 ORDER BY created_ms DESC LIMIT 1",
                vec![json!(group_id)],
            )
            .await?
            .first()
            .map(investigation))
    }

    /// The investigation a harness session belongs to. A session id is this
    /// worker's own construction, so at most one matches.
    pub async fn investigation_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<InvestigationSummaryV1>, SentinelError> {
        Ok(self
            .db()
            .query(
                "SELECT * FROM sentinel_investigations WHERE session_id = ? \
                 ORDER BY created_ms DESC LIMIT 1",
                vec![json!(session_id)],
            )
            .await?
            .first()
            .map(investigation))
    }

    pub async fn latest_investigation(
        &self,
        group_id: &str,
    ) -> Result<Option<InvestigationSummaryV1>, SentinelError> {
        Ok(self
            .db()
            .query(
                "SELECT * FROM sentinel_investigations WHERE group_id = ? \
                 ORDER BY created_ms DESC LIMIT 1",
                vec![json!(group_id)],
            )
            .await?
            .first()
            .map(investigation))
    }

    /// Every investigation still waiting on the harness, for the recovery
    /// pass that runs when a doorbell never arrives.
    pub async fn running_investigations(
        &self,
        limit: usize,
    ) -> Result<Vec<InvestigationSummaryV1>, SentinelError> {
        Ok(self
            .db()
            .query(
                "SELECT * FROM sentinel_investigations WHERE status = 'running' \
                 ORDER BY created_ms ASC LIMIT ?",
                vec![json!(limit as i64)],
            )
            .await?
            .iter()
            .map(investigation)
            .collect())
    }

    pub async fn list_investigations(
        &self,
        group_id: Option<&str>,
        statuses: &[InvestigationStatusV1],
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<InvestigationSummaryV1>, u64), SentinelError> {
        let mut where_sql = String::from("1 = 1");
        let mut params: Vec<Value> = Vec::new();
        if let Some(group_id) = group_id {
            where_sql.push_str(" AND group_id = ?");
            params.push(json!(group_id));
        }
        if !statuses.is_empty() {
            let slots = vec!["?"; statuses.len()].join(", ");
            where_sql.push_str(&format!(" AND status IN ({slots})"));
            params.extend(statuses.iter().map(|status| json!(status.as_str())));
        }
        let total = self
            .db()
            .query(
                &format!("SELECT COUNT(*) AS total FROM sentinel_investigations WHERE {where_sql}"),
                params.clone(),
            )
            .await?
            .first()
            .and_then(|row| row.get("total"))
            .and_then(Value::as_i64)
            .unwrap_or(0) as u64;
        let mut page = params;
        page.push(json!(limit as i64));
        page.push(json!(offset as i64));
        let rows = self
            .db()
            .query(
                &format!(
                    "SELECT * FROM sentinel_investigations WHERE {where_sql} \
                     ORDER BY created_ms DESC LIMIT ? OFFSET ?"
                ),
                page,
            )
            .await?;
        Ok((rows.iter().map(investigation).collect(), total))
    }

    /// Set named columns on one investigation.
    pub async fn update_investigation(
        &self,
        investigation_id: &str,
        set_sql: &str,
        mut params: Vec<Value>,
    ) -> Result<(), SentinelError> {
        params.push(json!(investigation_id));
        self.db()
            .execute(
                &format!("UPDATE sentinel_investigations SET {set_sql} WHERE id = ?"),
                params,
            )
            .await?;
        Ok(())
    }

    /// How many diagnoses this investigation has already recorded.
    pub async fn diagnosis_count(&self, investigation_id: &str) -> Result<u64, SentinelError> {
        Ok(self
            .db()
            .query(
                "SELECT COUNT(*) AS total FROM sentinel_diagnoses WHERE investigation_id = ?",
                vec![json!(investigation_id)],
            )
            .await?
            .first()
            .and_then(|row| row.get("total"))
            .and_then(Value::as_i64)
            .unwrap_or(0) as u64)
    }

    /// Newest first: the head of this list is the one in force.
    pub async fn diagnoses_for_group(
        &self,
        group_id: &str,
        limit: usize,
    ) -> Result<Vec<DiagnosisRecordV1>, SentinelError> {
        Ok(self.diagnoses_page(group_id, 0, limit).await?.0)
    }

    /// One page of a group's diagnoses, with how many there are in all. The
    /// count is what tells a reader that the one they are looking at
    /// superseded something.
    ///
    /// Ordered by time **and then by id**: a millisecond holds more than one
    /// recording when somebody asks twice in a row, and the id is a v7 uuid,
    /// so it breaks the tie in creation order instead of arbitrarily.
    pub async fn diagnoses_page(
        &self,
        group_id: &str,
        offset: u32,
        limit: usize,
    ) -> Result<(Vec<DiagnosisRecordV1>, u64), SentinelError> {
        let total = self
            .db()
            .query(
                "SELECT COUNT(*) AS total FROM sentinel_diagnoses WHERE group_id = ?",
                vec![json!(group_id)],
            )
            .await?
            .first()
            .and_then(|row| row.get("total"))
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0) as u64;
        let rows = self
            .db()
            .query(
                "SELECT d.*, i.session_id AS session_id, i.model AS model \
                 FROM sentinel_diagnoses d \
                 JOIN sentinel_investigations i ON i.id = d.investigation_id \
                 WHERE d.group_id = ? ORDER BY d.created_ms DESC, d.id DESC LIMIT ? OFFSET ?",
                vec![json!(group_id), json!(limit as i64), json!(offset as i64)],
            )
            .await?;
        Ok((rows.iter().map(diagnosis_record).collect(), total))
    }

    pub async fn diagnoses_for_investigation(
        &self,
        investigation_id: &str,
    ) -> Result<Vec<DiagnosisRecordV1>, SentinelError> {
        Ok(self
            .db()
            .query(
                "SELECT d.*, i.session_id AS session_id, i.model AS model \
                 FROM sentinel_diagnoses d \
                 JOIN sentinel_investigations i ON i.id = d.investigation_id \
                 WHERE d.investigation_id = ? ORDER BY d.created_ms DESC, d.id DESC",
                vec![json!(investigation_id)],
            )
            .await?
            .iter()
            .map(diagnosis_record)
            .collect())
    }

    pub async fn diagnosis_by_id(
        &self,
        diagnosis_id: &str,
    ) -> Result<Option<DiagnosisRecordV1>, SentinelError> {
        Ok(self
            .db()
            .query(
                "SELECT d.*, i.session_id AS session_id, i.model AS model \
                 FROM sentinel_diagnoses d \
                 JOIN sentinel_investigations i ON i.id = d.investigation_id \
                 WHERE d.id = ?",
                vec![json!(diagnosis_id)],
            )
            .await?
            .first()
            .map(diagnosis_record))
    }

    /// Write the diagnosis and move the group in one transaction, retrying
    /// when the group changed under the decision.
    ///
    /// The transition is computed from the state read inside the same round,
    /// so a resolve that landed a moment earlier wins: the row is written,
    /// the group stays resolved, and the response says so.
    pub async fn record_diagnosis(
        &self,
        investigation_id: &str,
        group_id: &str,
        turn_id: Option<&str>,
        source: DiagnosisSourceV1,
        diagnosis: &DiagnosisV1,
    ) -> Result<DiagnosisOutcome, SentinelError> {
        let payload = serde_json::to_string(diagnosis)
            .map_err(|error| SentinelError::invalid(error.to_string()))?;
        let version = self.diagnosis_count(investigation_id).await? + 1;

        for _ in 0..super::CAS_ATTEMPTS {
            let group = self
                .group_by_id(group_id)
                .await?
                .ok_or_else(|| SentinelError::NotFound(format!("group {group_id}")))?;
            let transition = lifecycle::on_record(&group.state);
            let diagnosis_id = ids::diagnosis_id();
            let now = ids::now_ms();

            let steps = vec![
                Statement::new(
                    "INSERT INTO sentinel_diagnoses \
                     (id, investigation_id, group_id, turn_id, source, recorded_by, diagnosis, \
                      valid, created_ms) \
                     VALUES (?, ?, ?, ?, ?, 'agent', ?, 1, ?)",
                    vec![
                        json!(diagnosis_id),
                        json!(investigation_id),
                        json!(group_id),
                        json!(turn_id),
                        json!(source.as_str()),
                        json!(payload),
                        json!(now),
                    ],
                ),
                Statement::new(
                    "UPDATE sentinel_groups SET status = ?, diagnosis_id = ?, updated_ms = ? \
                     WHERE id = ? AND updated_ms = ? RETURNING id",
                    vec![
                        json!(transition.status.as_str()),
                        json!(diagnosis_id),
                        json!(now),
                        json!(group_id),
                        json!(group.updated_ms),
                    ],
                ),
            ];
            let mut steps = steps;
            if transition.moved(group.state.status) {
                steps.push(super::transition_statement(
                    group_id,
                    Some(group.state.status),
                    transition.status,
                    transition.reason,
                    super::Actor::Agent,
                    now,
                ));
            }
            let results = self.db().transaction(&steps).await?;
            let applied = results
                .get(1)
                .is_some_and(|step| step.affected_rows > 0 || !step.rows.is_empty());
            if applied {
                return Ok(DiagnosisOutcome {
                    diagnosis_id,
                    version,
                    group_status: transition.status,
                    group_moved: transition.moved(group.state.status),
                });
            }
        }
        Err(SentinelError::dependency(
            "the group changed under every attempt to record the diagnosis",
        ))
    }

    pub async fn investigation_counts(&self) -> Result<InvestigationCountsV1, SentinelError> {
        let rows = self
            .db()
            .query(
                "SELECT status, COUNT(*) AS total FROM sentinel_investigations GROUP BY status",
                vec![],
            )
            .await?;
        let mut counts = InvestigationCountsV1::default();
        for row in &rows {
            let total = number(row, "total").unwrap_or_default().max(0) as u64;
            match text(row, "status").as_deref() {
                Some("running") => counts.running += total,
                // A session somebody can still talk to: the chat-mode ones
                // and the first passes that ended without closing the door.
                Some("open") | Some("completed") => counts.open_sessions += total,
                _ => {}
            }
        }
        counts.open_sessions += counts.running;
        Ok(counts)
    }
}

/// Whether a write failed because a unique constraint refused it — here,
/// always the partial index that allows one running pass per group.
fn is_conflict(error: &SentinelError) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("unique") || message.contains("constraint")
}

fn investigation(row: &NamedRow) -> InvestigationSummaryV1 {
    InvestigationSummaryV1 {
        id: text(row, "id").unwrap_or_default(),
        group_id: text(row, "group_id").unwrap_or_default(),
        occurrence_id: text(row, "occurrence_id").unwrap_or_default(),
        session_id: text(row, "session_id").unwrap_or_default(),
        mode: match text(row, "mode").as_deref() {
            Some("chat") => InvestigationModeV1::Chat,
            _ => InvestigationModeV1::Assisted,
        },
        first_pass_turn_id: text(row, "first_pass_turn_id"),
        model: text(row, "model").unwrap_or_default(),
        provider: text(row, "provider"),
        repository_id: text(row, "repository_id"),
        checkout_ref: text(row, "checkout_ref"),
        investigated_version: text(row, "investigated_version"),
        status: InvestigationStatusV1::parse(text(row, "status").as_deref().unwrap_or("running")),
        error: text(row, "error"),
        turns: number(row, "turns").map(|value| value.max(0) as u64),
        duration_ms: number(row, "duration_ms"),
        cost_usd: row.get("cost_usd").and_then(Value::as_f64),
        created_ms: number(row, "created_ms").unwrap_or_default(),
        finished_ms: number(row, "finished_ms"),
    }
}

fn diagnosis_record(row: &NamedRow) -> DiagnosisRecordV1 {
    let raw = text(row, "diagnosis");
    let parsed = raw
        .as_deref()
        .and_then(|json| serde_json::from_str::<DiagnosisV1>(json).ok());
    DiagnosisRecordV1 {
        id: text(row, "id").unwrap_or_default(),
        investigation_id: text(row, "investigation_id").unwrap_or_default(),
        group_id: text(row, "group_id").unwrap_or_default(),
        session_id: text(row, "session_id").unwrap_or_default(),
        turn_id: text(row, "turn_id"),
        source: match text(row, "source").as_deref() {
            Some("conversation") => DiagnosisSourceV1::Conversation,
            _ => DiagnosisSourceV1::FirstPass,
        },
        model: text(row, "model").unwrap_or_default(),
        created_ms: number(row, "created_ms").unwrap_or_default(),
        // A payload that no longer parses keeps its text: the shape can
        // change under a stored row, and losing the words would be worse
        // than showing them unstructured.
        valid: parsed.is_some(),
        raw_result: if parsed.is_some() { None } else { raw },
        diagnosis: parsed,
    }
}
