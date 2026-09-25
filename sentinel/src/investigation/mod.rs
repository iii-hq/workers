//! Investigations: a harness session beside the group's page.
//!
//! An investigation is not a job that returns a result. It is a conversation
//! somebody watches, and the structured part of it — the diagnosis — arrives
//! as a **function call the agent makes**, not as the turn's output. That is
//! the whole design: nothing forces the model into JSON, so the person can
//! interrupt and get prose back, while the record stays typed and versioned.
//!
//! Three rules are enforced here rather than hoped for:
//!
//! - **The row exists before the harness is told anything.** The doorbell can
//!   arrive before a `send` returns, and a diagnosis recorded against an
//!   investigation that is not in the table yet would be refused.
//! - **One running first pass per group.** The partial unique index is the
//!   guard; a second `investigate` gets the existing session back.
//! - **Identity never comes from the payload.** Who is recording is read from
//!   the invocation's baggage, which the agent cannot write.

pub mod message;
pub mod proxies;
pub mod record;

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ingest::CheckoutVersions;
use crate::store::investigations::InvestigationWrite;
use crate::store::{Db, Store};
use crate::{
    ids, lifecycle, ConfigCell, DoorbellResponseV1, GroupStateResponseV1, GroupStatusV1,
    InvestigateRequestV1, InvestigateResponseV1, InvestigationCancelRequestV1,
    InvestigationChangedEventV1, InvestigationChangedOpV1, InvestigationGetRequestV1,
    InvestigationGetResponseV1, InvestigationModeV1, InvestigationStatusV1, InvestigationSummaryV1,
    InvestigationsListRequestV1, InvestigationsListResponseV1, NamedRow, SentinelError, Statement,
    TurnCompletedEventV1,
};

use message::{MessageContext, PreviousOccurrence};

/// Running investigations reconciled per recovery pass. A ceiling, not a
/// budget: more than this waiting at once means the harness is wedged, and
/// the next pass takes the rest.
const RECONCILE_BATCH: usize = 25;
const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

/// What a turn looked like when the harness was last asked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TurnStatus {
    pub turn_id: Option<String>,
    pub status: String,
    /// The harness is waiting on something and will wake itself.
    pub expects_wake: bool,
    pub error: Option<String>,
}

impl TurnStatus {
    /// Whether the harness still owes this turn an answer.
    pub fn in_flight(&self) -> bool {
        self.expects_wake || matches!(self.status.as_str(), "running" | "awaiting_functions")
    }
}

/// What a turn cost, after the fact. Informative only.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TurnMetrics {
    pub turns: Option<u64>,
    pub duration_ms: Option<i64>,
    pub cost_usd: Option<f64>,
}

/// What `harness::send` answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendOutcome {
    pub session_id: String,
    pub turn_id: String,
}

/// The harness and session-manager surface an investigation needs. A trait so
/// every path through this module can be tested without a live agent.
#[async_trait]
pub trait Harness: Send + Sync {
    /// Open the filesystem jail on one checkout. Best effort: a refused grant
    /// costs the agent its source, not the investigation.
    async fn grant_filesystem(&self, session_id: &str, root: &str);
    async fn send(&self, request: Value) -> Result<SendOutcome, SentinelError>;
    /// `None` when the harness has never heard of the session.
    async fn status(&self, session_id: &str) -> Result<Option<TurnStatus>, SentinelError>;
    async fn stop(&self, session_id: &str, turn_id: &str) -> Result<(), SentinelError>;
    async fn metrics(&self, root_session_id: &str) -> TurnMetrics;
    /// Create the session without running anything.
    async fn ensure_session(
        &self,
        session_id: &str,
        title: &str,
        metadata: Value,
    ) -> Result<(), SentinelError>;
    /// Put one `user` entry in the transcript.
    async fn append_message(
        &self,
        session_id: &str,
        entry_id: &str,
        text: &str,
        origin: Value,
    ) -> Result<(), SentinelError>;
}

/// Something the console should hear about. Returned rather than emitted, so
/// the decision logic stays free of the client.
#[derive(Debug, Clone, PartialEq)]
pub enum Announcement {
    Group(GroupStateResponseV1),
    Investigation(InvestigationChangedEventV1),
}

/// A result with the events it produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome<T> {
    pub value: T,
    pub events: Vec<Announcement>,
}

impl<T> Outcome<T> {
    fn quiet(value: T) -> Self {
        Self {
            value,
            events: Vec::new(),
        }
    }
}

pub struct Investigations<D: Db> {
    store: Arc<Store<D>>,
    harness: Arc<dyn Harness>,
    checkouts: Arc<dyn CheckoutVersions>,
    config: ConfigCell,
}

impl<D: Db> Investigations<D> {
    pub fn new(
        store: Arc<Store<D>>,
        harness: Arc<dyn Harness>,
        checkouts: Arc<dyn CheckoutVersions>,
        config: ConfigCell,
    ) -> Self {
        Self {
            store,
            harness,
            checkouts,
            config,
        }
    }

    pub(crate) fn store(&self) -> &Arc<Store<D>> {
        &self.store
    }

    /// Open an investigation, or hand back the one already running.
    pub async fn investigate(
        &self,
        request: InvestigateRequestV1,
    ) -> Result<Outcome<InvestigateResponseV1>, SentinelError> {
        let config = self.config.read().await.clone();
        let group = self.group_row(&request.group_id).await?;

        let mode = request.mode.unwrap_or_default();
        // One running first pass per group, and one seated chat session:
        // clicking twice reopens what is there rather than starting a second
        // conversation about the same failure.
        if let Some(existing) = self.open_investigation(&request.group_id, mode).await? {
            return Ok(Outcome::quiet(InvestigateResponseV1 {
                investigation_id: existing.id,
                session_id: existing.session_id,
                first_pass_turn_id: existing.first_pass_turn_id,
                existing: true,
            }));
        }

        let occurrence = self
            .occurrence_with_evidence(&request.group_id, request.occurrence_id.as_deref())
            .await?;
        let model = request
            .model
            .clone()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                Some(config.investigation.model.clone()).filter(|value| !value.trim().is_empty())
            })
            .ok_or_else(|| {
                SentinelError::NoModel(
                    "no model in the request and none configured under investigation.model".into(),
                )
            })?;
        let provider = request
            .provider
            .clone()
            .or_else(|| config.investigation.provider.clone())
            // A blank provider is not a provider. It reaches the turn as an
            // empty string otherwise, and the router has to guess.
            .filter(|value| !value.trim().is_empty());

        let investigation_id = ids::investigation_id();
        let session_id = ids::investigation_session_id(&investigation_id);
        let service_name = text(&group, "service_name").unwrap_or_default();
        let repository = config.repository_for_worker(&service_name).cloned();
        let checkout_ref = match &repository {
            Some(_) => self.checkouts.version_for(&service_name).await,
            None => None,
        };
        let worker_version = text(&occurrence, "worker_version");

        let write = InvestigationWrite {
            id: investigation_id.clone(),
            group_id: request.group_id.clone(),
            occurrence_id: text(&occurrence, "id").unwrap_or_default(),
            session_id: session_id.clone(),
            mode,
            model: model.clone(),
            provider: provider.clone(),
            repository_id: repository.as_ref().map(|repo| repo.id.clone()),
            checkout_ref: checkout_ref.clone(),
            investigated_version: worker_version.clone(),
            status: match mode {
                InvestigationModeV1::Assisted => InvestigationStatusV1::Running,
                InvestigationModeV1::Chat => InvestigationStatusV1::Open,
            },
            requested_by: request._caller_worker_id.clone(),
            created_ms: ids::now_ms(),
        };
        // The row goes in before the harness hears anything: a doorbell or a
        // `record` can beat the send's own answer back to us.
        if !self.store.insert_investigation(&write).await? {
            let existing = self
                .open_investigation(&request.group_id, mode)
                .await?
                .ok_or_else(|| {
                    SentinelError::dependency("an investigation was refused with none running")
                })?;
            return Ok(Outcome::quiet(InvestigateResponseV1 {
                investigation_id: existing.id,
                session_id: existing.session_id,
                first_pass_turn_id: existing.first_pass_turn_id,
                existing: true,
            }));
        }

        let context = self
            .message_context(
                &group,
                &occurrence,
                repository.clone(),
                checkout_ref.clone(),
            )
            .await?;
        let title = format!("Sentinel: {}", context.title);
        let metadata = json!({
            "sentinel": true,
            "sentinel_group_id": request.group_id,
            "sentinel_investigation_id": investigation_id,
        });

        let mut events = vec![Announcement::Investigation(InvestigationChangedEventV1 {
            op: InvestigationChangedOpV1::Created,
            investigation_id: investigation_id.clone(),
            group_id: request.group_id.clone(),
            session_id: session_id.clone(),
            status: write.status,
            error: None,
        })];

        let first_pass_turn_id = match mode {
            InvestigationModeV1::Chat => {
                let text =
                    message::chat_evidence(&context, config.evidence.message_max_bytes as usize);
                let seated = async {
                    self.harness
                        .ensure_session(&session_id, &title, metadata)
                        .await?;
                    self.harness
                        .append_message(
                            &session_id,
                            &format!("{investigation_id}:evidence"),
                            &text,
                            json!({
                                "sentinel_evidence": true,
                                "sentinel_group_id": request.group_id,
                                "sentinel_investigation_id": investigation_id,
                            }),
                        )
                        .await
                }
                .await;
                if let Err(error) = seated {
                    // An empty session is worse than none: it would sit in the
                    // sidebar offering a conversation about evidence it never
                    // received.
                    self.fail(&investigation_id, &error.to_string()).await?;
                    return Err(SentinelError::HarnessUnavailable(error.to_string()));
                }
                None
            }
            InvestigationModeV1::Assisted => {
                if let Some(repository) = &repository {
                    self.harness
                        .grant_filesystem(&session_id, &repository.path)
                        .await;
                }
                let body =
                    message::first_pass(&context, config.evidence.message_max_bytes as usize);
                let payload = send_request(
                    &session_id,
                    &body,
                    &model,
                    provider.as_deref(),
                    &format!("{investigation_id}:analysis"),
                    &title,
                    metadata,
                    repository.as_ref().map(|repo| repo.path.as_str()),
                );
                match self.harness.send(payload).await {
                    Ok(outcome) => {
                        self.store
                            .update_investigation(
                                &investigation_id,
                                "first_pass_turn_id = ?",
                                vec![json!(outcome.turn_id)],
                            )
                            .await?;
                        if let Some(moved) = self.mark_investigating(&request.group_id).await? {
                            events.push(Announcement::Group(moved));
                        }
                        Some(outcome.turn_id)
                    }
                    Err(error) => {
                        let detail = error.to_string();
                        self.fail(&investigation_id, &detail).await?;
                        return Err(SentinelError::HarnessUnavailable(detail));
                    }
                }
            }
        };

        Ok(Outcome {
            value: InvestigateResponseV1 {
                investigation_id,
                session_id,
                first_pass_turn_id,
                existing: false,
            },
            events,
        })
    }

    /// The investigation this mode would reopen: the running first pass, or
    /// the chat session already seated with this group's evidence.
    async fn open_investigation(
        &self,
        group_id: &str,
        mode: InvestigationModeV1,
    ) -> Result<Option<InvestigationSummaryV1>, SentinelError> {
        if let Some(running) = self.store.running_investigation(group_id).await? {
            return Ok(Some(running));
        }
        if mode != InvestigationModeV1::Chat {
            return Ok(None);
        }
        Ok(self
            .store
            .latest_investigation(group_id)
            .await?
            .filter(|investigation| {
                investigation.mode == InvestigationModeV1::Chat
                    && investigation.status == InvestigationStatusV1::Open
            }))
    }

    /// Close a record that never started, naming why.
    async fn fail(&self, investigation_id: &str, detail: &str) -> Result<(), SentinelError> {
        self.store
            .update_investigation(
                investigation_id,
                "status = ?, error = ?, finished_ms = ?",
                vec![
                    json!(InvestigationStatusV1::Failed.as_str()),
                    json!(detail),
                    json!(ids::now_ms()),
                ],
            )
            .await
    }

    pub async fn get(
        &self,
        request: InvestigationGetRequestV1,
    ) -> Result<InvestigationGetResponseV1, SentinelError> {
        let investigation = self
            .store
            .investigation_by_id(&request.investigation_id)
            .await?
            .ok_or_else(|| {
                SentinelError::NotFound(format!("investigation {}", request.investigation_id))
            })?;
        let diagnoses = self
            .store
            .diagnoses_for_investigation(&request.investigation_id)
            .await?;
        Ok(InvestigationGetResponseV1 {
            investigation,
            diagnoses,
        })
    }

    pub async fn list(
        &self,
        request: InvestigationsListRequestV1,
    ) -> Result<InvestigationsListResponseV1, SentinelError> {
        let (investigations, total) = self
            .store
            .list_investigations(
                request.group_id.as_deref(),
                &request.status.unwrap_or_default(),
                request.offset.unwrap_or(0),
                request.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT),
            )
            .await?;
        Ok(InvestigationsListResponseV1 {
            investigations,
            total,
        })
    }

    /// Stop the first pass. The group goes back where it came from unless
    /// something was recorded — a cancelled pass leaves no mark.
    pub async fn cancel(
        &self,
        request: InvestigationCancelRequestV1,
    ) -> Result<Outcome<InvestigationSummaryV1>, SentinelError> {
        let investigation = self
            .store
            .investigation_by_id(&request.investigation_id)
            .await?
            .ok_or_else(|| {
                SentinelError::NotFound(format!("investigation {}", request.investigation_id))
            })?;
        if !investigation.status.is_running() {
            return Ok(Outcome::quiet(investigation));
        }
        if let Some(turn_id) = &investigation.first_pass_turn_id {
            if let Err(error) = self.harness.stop(&investigation.session_id, turn_id).await {
                tracing::warn!(
                    investigation_id = investigation.id,
                    %error,
                    "could not stop the investigation turn; closing the record anyway"
                );
            }
        }
        let events = self
            .close(&investigation, InvestigationStatusV1::Cancelled, None)
            .await?;
        let updated = self
            .store
            .investigation_by_id(&request.investigation_id)
            .await?
            .unwrap_or(investigation);
        Ok(Outcome {
            value: updated,
            events,
        })
    }

    /// The doorbell. It says only "look again"; everything decided below is
    /// read from `harness::status`.
    pub async fn on_turn_completed(
        &self,
        event: TurnCompletedEventV1,
    ) -> Result<Outcome<DoorbellResponseV1>, SentinelError> {
        if !ids::is_investigation_session(&event.session_id) {
            return Ok(Outcome::quiet(DoorbellResponseV1 { handled: false }));
        }
        let Some(investigation) = self
            .store
            .investigation_by_session(&event.session_id)
            .await?
        else {
            return Ok(Outcome::quiet(DoorbellResponseV1 { handled: false }));
        };
        if !investigation.status.is_running() {
            return Ok(Outcome::quiet(DoorbellResponseV1 { handled: false }));
        }
        // Only the first pass is this worker's turn. Everything after it
        // belongs to the person having the conversation.
        if event.turn_id.is_some() && event.turn_id != investigation.first_pass_turn_id {
            return Ok(Outcome::quiet(DoorbellResponseV1 { handled: false }));
        }
        let events = self.finish_first_pass(&investigation).await?;
        Ok(Outcome {
            value: DoorbellResponseV1 { handled: true },
            events,
        })
    }

    /// Catch the investigations whose doorbell never rang.
    pub async fn reconcile(&self) -> Vec<Announcement> {
        let running = match self.store.running_investigations(RECONCILE_BATCH).await {
            Ok(running) => running,
            Err(error) => {
                tracing::warn!(%error, "could not read the running investigations");
                return Vec::new();
            }
        };
        let mut events = Vec::new();
        for investigation in running {
            match self.finish_first_pass(&investigation).await {
                Ok(mut produced) => events.append(&mut produced),
                Err(error) => tracing::warn!(
                    investigation_id = investigation.id,
                    %error,
                    "could not reconcile an investigation"
                ),
            }
        }
        events
    }

    /// Decide what a running first pass has become, and act on it once.
    async fn finish_first_pass(
        &self,
        investigation: &InvestigationSummaryV1,
    ) -> Result<Vec<Announcement>, SentinelError> {
        let Some(status) = self.harness.status(&investigation.session_id).await? else {
            // The harness has never heard of the session: the send failed in
            // a way that left no turn. Nothing to wait for.
            return self
                .close(
                    investigation,
                    InvestigationStatusV1::Failed,
                    Some("the harness has no record of this session".into()),
                )
                .await;
        };
        // A turn that is not ours says nothing about ours.
        if investigation.first_pass_turn_id.is_some()
            && status.turn_id.is_some()
            && status.turn_id != investigation.first_pass_turn_id
        {
            return Ok(Vec::new());
        }
        if status.in_flight() {
            return Ok(Vec::new());
        }

        let recorded = self.store.diagnosis_count(&investigation.id).await? > 0;
        match status.status.as_str() {
            "completed"
                if !recorded && !investigation_was_nudged(&self.store, investigation).await? =>
            {
                // It finished without recording anything. One nudge, ever.
                self.nudge(investigation).await
            }
            "completed" => {
                self.close(investigation, InvestigationStatusV1::Completed, None)
                    .await
            }
            "cancelled" => {
                self.close(
                    investigation,
                    InvestigationStatusV1::Cancelled,
                    status.error,
                )
                .await
            }
            "failed" => {
                self.close(investigation, InvestigationStatusV1::Failed, status.error)
                    .await
            }
            other => {
                tracing::warn!(
                    investigation_id = investigation.id,
                    status = other,
                    "harness::status reported a state this worker does not know"
                );
                Ok(Vec::new())
            }
        }
    }

    /// Ask once for the diagnosis the pass never wrote. The investigation
    /// stays running, now waiting on the nudge's own turn.
    async fn nudge(
        &self,
        investigation: &InvestigationSummaryV1,
    ) -> Result<Vec<Announcement>, SentinelError> {
        let config = self.config.read().await.clone();
        let payload = send_request(
            &investigation.session_id,
            &format!(
                "You finished without recording anything. Record what you have now with \
                 `sentinel::diagnosis::record` for group `{}` — if the evidence did not settle \
                 it, use `confidence: \"low\"` and list what was missing in `missing_evidence`.",
                investigation.group_id
            ),
            &investigation.model,
            investigation.provider.as_deref(),
            &format!("{}:nudge", investigation.id),
            "Sentinel",
            json!({ "sentinel": true, "sentinel_investigation_id": investigation.id }),
            config
                .repository_for_worker_id(investigation.repository_id.as_deref())
                .map(|repo| repo.path.as_str()),
        );
        match self.harness.send(payload).await {
            Ok(outcome) => {
                self.store
                    .update_investigation(
                        &investigation.id,
                        "nudged = 1, first_pass_turn_id = ?",
                        vec![json!(outcome.turn_id)],
                    )
                    .await?;
                Ok(Vec::new())
            }
            Err(error) => {
                // The nudge is a courtesy; failing it must not leave the
                // investigation running forever.
                tracing::warn!(investigation_id = investigation.id, %error, "the nudge failed");
                self.store
                    .update_investigation(&investigation.id, "nudged = 1", vec![])
                    .await?;
                self.close(investigation, InvestigationStatusV1::Completed, None)
                    .await
            }
        }
    }

    /// Write the final state, roll the group back if nothing was recorded,
    /// and collect the observed cost.
    async fn close(
        &self,
        investigation: &InvestigationSummaryV1,
        status: InvestigationStatusV1,
        error: Option<String>,
    ) -> Result<Vec<Announcement>, SentinelError> {
        let metrics = self.harness.metrics(&investigation.session_id).await;
        self.store
            .update_investigation(
                &investigation.id,
                "status = ?, error = ?, finished_ms = ?, turns = ?, duration_ms = ?, cost_usd = ?",
                vec![
                    json!(status.as_str()),
                    json!(error),
                    json!(ids::now_ms()),
                    json!(metrics.turns),
                    json!(metrics.duration_ms),
                    json!(metrics.cost_usd),
                ],
            )
            .await?;

        let recorded = self.store.diagnosis_count(&investigation.id).await? > 0;
        let mut events = Vec::new();
        if let Some(moved) = self
            .release_group(&investigation.group_id, recorded)
            .await?
        {
            events.push(Announcement::Group(moved));
        }
        events.push(Announcement::Investigation(InvestigationChangedEventV1 {
            op: InvestigationChangedOpV1::Finished,
            investigation_id: investigation.id.clone(),
            group_id: investigation.group_id.clone(),
            session_id: investigation.session_id.clone(),
            status,
            error: None,
        }));
        Ok(events)
    }

    /// Move the group to `investigating`, under compare-and-set. A group that
    /// cannot make that move — somebody resolved it a moment ago — keeps its
    /// state: the investigation still runs, it just does not relabel a
    /// decision somebody made.
    async fn mark_investigating(
        &self,
        group_id: &str,
    ) -> Result<Option<GroupStateResponseV1>, SentinelError> {
        for _ in 0..crate::store::CAS_ATTEMPTS {
            let Some(group) = self.store.group_by_id(group_id).await? else {
                return Ok(None);
            };
            let transition = match lifecycle::on_investigate(&group.state) {
                Ok(transition) => transition,
                Err(_) => return Ok(None),
            };
            if !transition.moved(group.state.status) {
                return Ok(None);
            }
            if self
                .apply_status(
                    group_id,
                    group.updated_ms,
                    group.state.status,
                    &transition,
                    true,
                )
                .await?
            {
                return Ok(Some(GroupStateResponseV1 {
                    group_id: group_id.to_string(),
                    status: transition.status,
                    reason: transition.reason,
                }));
            }
        }
        Ok(None)
    }

    /// The first pass ended: put the group back where it was, unless a
    /// diagnosis landed or somebody moved it meanwhile.
    async fn release_group(
        &self,
        group_id: &str,
        recorded: bool,
    ) -> Result<Option<GroupStateResponseV1>, SentinelError> {
        for _ in 0..crate::store::CAS_ATTEMPTS {
            let Some(group) = self.store.group_by_id(group_id).await? else {
                return Ok(None);
            };
            let transition = lifecycle::on_first_pass_end(&group.state, recorded);
            if !transition.moved(group.state.status) {
                return Ok(None);
            }
            if self
                .apply_status(
                    group_id,
                    group.updated_ms,
                    group.state.status,
                    &transition,
                    false,
                )
                .await?
            {
                return Ok(Some(GroupStateResponseV1 {
                    group_id: group_id.to_string(),
                    status: transition.status,
                    reason: transition.reason,
                }));
            }
        }
        Ok(None)
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_status(
        &self,
        group_id: &str,
        seen_updated_ms: i64,
        from: GroupStatusV1,
        transition: &crate::Transition,
        remember_previous: bool,
    ) -> Result<bool, SentinelError> {
        let set_sql = if remember_previous {
            "status = ?, previous_status = status, updated_ms = ?"
        } else {
            "status = ?, updated_ms = ?"
        };
        let now = ids::now_ms();
        let results = self
            .store
            .db()
            .transaction(&[
                Statement::new(
                    format!(
                        "UPDATE sentinel_groups SET {set_sql} WHERE id = ? AND updated_ms = ? \
                         RETURNING id"
                    ),
                    vec![
                        json!(transition.status.as_str()),
                        json!(now),
                        json!(group_id),
                        json!(seen_updated_ms),
                    ],
                ),
                crate::store::transition_statement(
                    group_id,
                    Some(from),
                    transition.status,
                    transition.reason,
                    crate::store::Actor::Investigation,
                    now,
                ),
            ])
            .await?;
        Ok(results
            .first()
            .is_some_and(|step| step.affected_rows > 0 || !step.rows.is_empty()))
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

    /// The occurrence to investigate: the one asked for, or the most recent
    /// that still has its evidence. Without evidence there is nothing to
    /// investigate from, so this refuses rather than opening an empty session.
    async fn occurrence_with_evidence(
        &self,
        group_id: &str,
        occurrence_id: Option<&str>,
    ) -> Result<NamedRow, SentinelError> {
        let row = match occurrence_id {
            Some(id) => self
                .store
                .db()
                .query(
                    "SELECT * FROM sentinel_occurrences WHERE id = ? AND group_id = ?",
                    vec![json!(id), json!(group_id)],
                )
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    SentinelError::NotFound(format!("occurrence {id} in group {group_id}"))
                })?,
            None => self
                .store
                .db()
                .query(
                    "SELECT * FROM sentinel_occurrences WHERE group_id = ? AND evidence IS NOT NULL \
                     ORDER BY at_ms DESC LIMIT 1",
                    vec![json!(group_id)],
                )
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    SentinelError::NoEvidence(format!(
                        "no occurrence of {group_id} still has its evidence"
                    ))
                })?,
        };
        if text(&row, "evidence").is_none() {
            return Err(SentinelError::NoEvidence(format!(
                "the evidence of occurrence {} was pruned",
                text(&row, "id").unwrap_or_default()
            )));
        }
        Ok(row)
    }

    async fn message_context(
        &self,
        group: &NamedRow,
        occurrence: &NamedRow,
        repository: Option<crate::RepositoryConfigV1>,
        checkout_ref: Option<String>,
    ) -> Result<MessageContext, SentinelError> {
        let group_id = text(group, "id").unwrap_or_default();
        let occurrence_id = text(occurrence, "id").unwrap_or_default();
        let sessions_affected = self
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
            .unwrap_or(0)
            .max(0) as u64;
        let previous = self
            .store
            .db()
            .query(
                "SELECT at_ms, worker_version, message FROM sentinel_occurrences \
                 WHERE group_id = ? AND id != ? ORDER BY at_ms DESC LIMIT 3",
                vec![json!(group_id), json!(occurrence_id)],
            )
            .await?
            .iter()
            .map(|row| PreviousOccurrence {
                at_ms: number(row, "at_ms").unwrap_or_default(),
                worker_version: text(row, "worker_version"),
                message: text(row, "message").unwrap_or_default(),
            })
            .collect();

        Ok(MessageContext {
            group_id,
            title: text(group, "title").unwrap_or_default(),
            fingerprint: text(group, "fingerprint").unwrap_or_default(),
            service_name: text(group, "service_name").unwrap_or_default(),
            function_id: text(group, "function_id"),
            status: crate::service::status_of(text(group, "status").as_deref()),
            occurrence_count: number(group, "occurrence_count").unwrap_or_default().max(0) as u64,
            sessions_affected,
            first_seen_ms: number(group, "first_seen_ms").unwrap_or_default(),
            last_seen_ms: number(group, "last_seen_ms").unwrap_or_default(),
            message_sample: text(group, "message_sample").unwrap_or_default(),
            occurrence_at_ms: number(occurrence, "at_ms").unwrap_or_default(),
            worker_version: text(occurrence, "worker_version"),
            occurrence_id,
            checkout_ref,
            repository,
            evidence: text(occurrence, "evidence")
                .and_then(|json| serde_json::from_str(&json).ok()),
            previous,
        })
    }
}

/// Whether the single nudge has already been spent.
async fn investigation_was_nudged<D: Db>(
    store: &Store<D>,
    investigation: &InvestigationSummaryV1,
) -> Result<bool, SentinelError> {
    Ok(store
        .db()
        .query(
            "SELECT nudged FROM sentinel_investigations WHERE id = ?",
            vec![json!(investigation.id)],
        )
        .await?
        .first()
        .and_then(|row| row.get("nudged"))
        .map(|value| value.as_i64().unwrap_or(0) != 0 || value.as_bool().unwrap_or(false))
        .unwrap_or(false))
}

/// The `harness::send` payload for one turn.
///
/// There is deliberately no `options.output`: a structured-output turn makes
/// *every* assistant message JSON, so the person watching would get JSON back
/// when they typed. The structure lives in the function the agent calls.
#[allow(clippy::too_many_arguments)]
fn send_request(
    session_id: &str,
    message: &str,
    model: &str,
    provider: Option<&str>,
    idempotency_key: &str,
    title: &str,
    metadata: Value,
    filesystem_root: Option<&str>,
) -> Value {
    json!({
        "session_id": session_id,
        "message": message,
        "model": model,
        "provider": provider,
        "idempotency_key": idempotency_key,
        "session": {
            "title": title,
            "kind": "automation",
            "metadata": metadata,
        },
        "options": {
            "system_prompt": message::SYSTEM_PROMPT,
            "system_prompt_strategy": "override",
            "functions": {
                "allow": crate::functions::INVESTIGATION_ALLOW,
                "deny": crate::functions::INVESTIGATION_DENY,
                "expose": "agent_trigger",
            },
            "metadata": match filesystem_root {
                Some(root) => json!({ "fs_scope": { "root": root } }),
                None => json!({}),
            },
        },
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_is_never_asked_for_structured_output() {
        let payload = send_request(
            "sentinel-inv-x",
            "body",
            "anthropic::claude-sonnet-5",
            Some("anthropic"),
            "inv:analysis",
            "Sentinel: boom",
            json!({}),
            Some("/repo"),
        );
        assert!(
            payload["options"].get("output").is_none(),
            "a structured-output turn would make every reply JSON, including to the person \
             watching: {payload}"
        );
        assert_eq!(payload["session"]["kind"], "automation");
        assert_eq!(payload["options"]["metadata"]["fs_scope"]["root"], "/repo");
        assert_eq!(payload["options"]["functions"]["expose"], "agent_trigger");
    }

    #[test]
    fn a_turn_without_a_checkout_grants_no_filesystem_scope() {
        let payload = send_request(
            "sentinel-inv-x",
            "body",
            "m",
            None,
            "inv:analysis",
            "Sentinel",
            json!({}),
            None,
        );
        assert_eq!(payload["options"]["metadata"], json!({}));
    }

    #[test]
    fn a_turn_in_flight_is_not_finished() {
        for status in ["running", "awaiting_functions"] {
            assert!(TurnStatus {
                status: status.into(),
                ..TurnStatus::default()
            }
            .in_flight());
        }
        assert!(TurnStatus {
            status: "completed".into(),
            expects_wake: true,
            ..TurnStatus::default()
        }
        .in_flight());
        assert!(!TurnStatus {
            status: "completed".into(),
            ..TurnStatus::default()
        }
        .in_flight());
    }
}
