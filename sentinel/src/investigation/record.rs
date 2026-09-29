//! `sentinel::diagnosis::record` — the one write an investigation can make.
//!
//! The identity of the caller is taken from the invocation's OTel baggage and
//! from nowhere else. The harness stamps `iii.session.id` on the turn, the
//! engine carries it into the invocation, and the SDK runs the handler inside
//! that context — none of which the agent can influence. A payload field
//! naming the investigation would be a field a model could get wrong or a
//! caller could forge, so the request has no such field and there is no
//! fallback if the baggage is missing.

use serde_json::json;

use super::{Announcement, Investigations, Outcome};
use crate::store::Db;
use crate::{
    ids, DiagnosisRecordRequestV1, DiagnosisRecordResponseV1, DiagnosisSourceV1,
    GroupChangeReasonV1, GroupStateResponseV1, InvestigationChangedEventV1,
    InvestigationChangedOpV1, SentinelError,
};

/// Who is calling, as the runtime reported it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Caller {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
}

impl<D: Db> Investigations<D> {
    pub async fn record(
        &self,
        caller: Caller,
        request: DiagnosisRecordRequestV1,
    ) -> Result<Outcome<DiagnosisRecordResponseV1>, SentinelError> {
        let session_id = caller
            .session_id
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                SentinelError::NoInvestigation(
                "this call carries no session; a diagnosis can only be recorded from inside an \
                 investigation"
                    .into(),
            )
            })?;
        if !ids::is_investigation_session(&session_id) {
            return Err(SentinelError::NoInvestigation(format!(
                "session {session_id} is not a sentinel investigation"
            )));
        }
        let investigation = self
            .store()
            .investigation_by_session(&session_id)
            .await?
            .ok_or_else(|| {
                SentinelError::NoInvestigation(format!(
                    "session {session_id} names no investigation this worker opened"
                ))
            })?;
        if investigation.group_id != request.group_id {
            return Err(SentinelError::NotThisGroup(format!(
                "this session investigates {}, not {}",
                investigation.group_id, request.group_id
            )));
        }

        // `first_pass` only while the automatic pass is the turn in progress.
        // Everything after it is the conversation, whoever asked for it.
        let source = if investigation.status.is_running()
            && caller.turn_id.is_some()
            && caller.turn_id == investigation.first_pass_turn_id
        {
            DiagnosisSourceV1::FirstPass
        } else {
            DiagnosisSourceV1::Conversation
        };

        let outcome = self
            .store()
            .record_diagnosis(
                &investigation.id,
                &investigation.group_id,
                caller.turn_id.as_deref(),
                source,
                &request.diagnosis,
            )
            .await?;

        let mut events = vec![Announcement::Investigation(InvestigationChangedEventV1 {
            op: InvestigationChangedOpV1::Recorded,
            investigation_id: investigation.id.clone(),
            group_id: investigation.group_id.clone(),
            session_id: investigation.session_id.clone(),
            status: investigation.status,
            error: None,
        })];
        if outcome.group_moved {
            events.push(Announcement::Group(GroupStateResponseV1 {
                group_id: investigation.group_id.clone(),
                status: outcome.group_status,
                reason: Some(GroupChangeReasonV1::Diagnosed),
            }));
        }

        tracing::info!(
            investigation_id = investigation.id,
            group_id = investigation.group_id,
            version = outcome.version,
            source = source.as_str(),
            status = outcome.group_status.as_str(),
            "a diagnosis was recorded"
        );
        let _ = json!({});

        Ok(Outcome {
            value: DiagnosisRecordResponseV1 {
                diagnosis_id: outcome.diagnosis_id,
                version: outcome.version,
                group_status: outcome.group_status,
            },
            events,
        })
    }
}
