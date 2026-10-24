//! The stages between a measured baseline (or a triaged execution) and the
//! candidate's source validation: the semantic triage of ambiguous
//! findings, the bounded read-only proposal, the isolated candidate actor
//! and the dispatch of its validation. Each reserves its work before it
//! dispatches, persists the actor's stable identity before starting it, and
//! is polled by the sweep while it runs.

use std::collections::BTreeMap;

use judge_contract::{EvaluateRequest, EvaluateResponse, Evaluation, Question};
use serde_json::{json, Value};

use crate::candidate::{
    self, CandidateConfigV1, CandidateError, CandidateEvidenceV1, CandidateStatusV1,
    ProposalInputV1,
};
use crate::contract::*;
use crate::error::EvalError;
use crate::runtime::{
    call, dispatch, end, fail, reserve, settle, transition, Deps, BUS_TIMEOUT_MS,
};
use crate::{cost, ids, state};

const JUDGE_TIMEOUT_MS: u64 = 60_000;
const TRIAGE_EVALUATION: &str = "findings";
const TRIAGE_QUESTION: &str = "opportunity";
/// Native evidence entries handed to the proposal.
const MAX_EVIDENCE: usize = 20;
/// The longest a candidate push may take before its outcome is unknown.
const PUBLISH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn candidate_config(record: &CycleRecordV1) -> Option<&CandidateConfigV1> {
    record.config.as_ref()?.config.candidate.as_ref()
}

/// The candidate commit the cycle validated, as the module recorded it.
pub fn candidate_sha(record: &CycleRecordV1) -> Option<String> {
    Some(
        record
            .candidate
            .as_ref()?
            .candidate
            .as_ref()?
            .commit
            .as_ref()?
            .candidate_sha
            .clone(),
    )
}

fn findings_of(record: &CycleRecordV1, pertinent_only: bool) -> Vec<(String, FindingV1)> {
    record
        .analyses
        .iter()
        .flat_map(|analysis| {
            analysis
                .findings
                .iter()
                .filter(move |finding| !pertinent_only || finding.class.is_pertinent())
                .map(move |finding| (analysis.execution_id.clone(), finding.clone()))
        })
        .collect()
}

/// Stops after a reason the cycle cannot go past; `code` decides how.
async fn stop(
    deps: &Deps,
    record: &CycleRecordV1,
    code: OutcomeCodeV1,
    detail: String,
) -> Result<Option<CycleRecordV1>, EvalError> {
    end(deps, record, code, &detail).await
}

// ---------------------------------------------------------------------------
// Triage
// ---------------------------------------------------------------------------

fn triage_question() -> Question {
    Question::Choice {
        instructions: "These findings come from the deterministic first pass over finished \
            Harness E2E runs. Decide, for the group as a whole, whether they show a relevant \
            opportunity to improve the Harness worth an isolated candidate change."
            .into(),
        criteria: BTreeMap::from([
            (
                "relevant_opportunity".into(),
                "The runs show a concrete, repeatable Harness behavior worth changing.".into(),
            ),
            (
                "expected_behavior".into(),
                "The behavior is what the scenario expects; nothing to change.".into(),
            ),
            (
                "insufficient_evidence".into(),
                "The evidence cannot tell; no change should be attempted.".into(),
            ),
        ]),
    }
}

/// Decides whether a source execution's pertinent findings go on to
/// candidate work. A clear deterministic defect needs no judgment; Sentinel's triage of the same occurrences is reused;
/// only an ambiguous improvement opportunity is asked, once, of
/// `judge::evaluate`. A request found from before a restart is never sent
/// again: its semantic step is unknown.
pub async fn triage_stage(
    deps: &Deps,
    record: &CycleRecordV1,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let pertinent = findings_of(record, true);
    let fingerprints: Vec<String> = pertinent
        .iter()
        .map(|(_, finding)| finding.fingerprint.clone())
        .collect();
    let reused: Vec<Value> = record
        .analyses
        .iter()
        .filter_map(|analysis| analysis.sentinel.as_ref())
        .flat_map(|sentinel| sentinel.results.iter())
        .filter(|result| !result["triage"].is_null())
        .cloned()
        .collect();
    let base = |source, outcome, reason: String| TriageV1 {
        source,
        outcome,
        findings: fingerprints.clone(),
        reason,
        request_id: None,
        question: None,
        evidence_digest: None,
        model: None,
        answer: None,
        usage: None,
        reused: reused.clone(),
    };
    let proceed = |triage: TriageV1| async move {
        let detail = triage.reason.clone();
        transition(
            deps,
            &record.cycle_id,
            record.step,
            CycleStatusV1::Running,
            CycleStageV1::Baseline,
            &detail,
            |next| {
                next.triage = Some(triage.clone());
                Ok(())
            },
        )
        .await
    };
    let finish = |triage: TriageV1, code: OutcomeCodeV1| async move {
        let detail = triage.reason.clone();
        transition(
            deps,
            &record.cycle_id,
            record.step,
            CycleStatusV1::Completed,
            CycleStageV1::Done,
            &detail,
            |next| {
                next.triage = Some(triage.clone());
                next.outcome = Some(CycleOutcomeV1 {
                    code,
                    detail: detail.clone(),
                });
                Ok(())
            },
        )
        .await
    };
    let deterministic = pertinent
        .iter()
        .find(|(_, finding)| finding.class == FindingClassV1::Defect);
    if let Some((execution_id, finding)) = deterministic {
        return proceed(base(
            TriageSourceV1::Deterministic,
            TriageOutcomeV1::Proceed,
            format!(
                "a deterministic {:?} in {execution_id} needs no judgment: {}",
                finding.class, finding.observation
            ),
        ))
        .await;
    }
    if let Some(previous) = record
        .triage
        .as_ref()
        .filter(|triage| triage.request_id.is_some())
    {
        let mut unknown = previous.clone();
        unknown.outcome = TriageOutcomeV1::Unknown;
        unknown.reason =
            "the judge was asked before a restart and its answer was lost; it is not asked twice"
                .into();
        return finish(unknown, OutcomeCodeV1::Inconclusive).await;
    }
    let Some(judge) = record
        .config
        .as_ref()
        .and_then(|stored| stored.config.judge.clone())
    else {
        return finish(
            base(
                TriageSourceV1::Jev,
                TriageOutcomeV1::Unknown,
                "only ambiguous improvement opportunities, and no judge is configured (config.judge)"
                    .into(),
            ),
            OutcomeCodeV1::Inconclusive,
        )
        .await;
    };
    let work = CycleWorkV1 {
        work_id: format!("{}:triage", record.cycle_id),
        kind: CycleWorkKindV1::Jev,
        estimated_usd: judge.estimated_usd,
        candidate: false,
    };
    if let cost::Reservation::OverLimit(detail) = reserve(deps, record, &work).await? {
        return end(deps, record, OutcomeCodeV1::Budget, &detail).await;
    }
    let state_value = json!({
        "objective": "Does this group of findings show a relevant Harness improvement opportunity?",
        "findings": pertinent.iter().map(|(execution_id, finding)| json!({
            "execution_id": execution_id, "class": finding.class, "signal": finding.signal,
            "observation": finding.observation})).collect::<Vec<_>>(),
    });
    let question = triage_question();
    let request_id = format!("{}-triage", record.cycle_id);
    let mut asked = base(
        TriageSourceV1::Jev,
        TriageOutcomeV1::Unknown,
        "asked the judge".into(),
    );
    asked.request_id = Some(request_id.clone());
    asked.question = Some(serde_json::to_value(&question)?);
    asked.evidence_digest = Some(ids::sha256_json(&state_value));
    // Persisted before the call: a restart never asks twice.
    let persisted = state::update_cycle(&deps.iii, &record.cycle_id, |stored| {
        if stored.step != record.step || stored.status.is_terminal() {
            return Ok((None, false));
        }
        let mut next = stored.clone();
        next.triage = Some(asked.clone());
        Ok((Some(next), true))
    })
    .await?
    .unwrap_or(false);
    if !persisted {
        return Ok(None);
    }
    let request = EvaluateRequest {
        options: Default::default(),
        request_id: Some(request_id),
        model: None,
        timeout_ms: JUDGE_TIMEOUT_MS,
        expires_at_unix_ms: None,
        evaluations: vec![Evaluation {
            id: TRIAGE_EVALUATION.into(),
            state: state_value,
            questions: BTreeMap::from([(TRIAGE_QUESTION.into(), question.clone())]),
        }],
    };
    let mut payload = serde_json::to_value(request)?;
    payload["provider"] = json!(judge.provider);
    let answered: Result<EvaluateResponse, EvalError> = call(
        deps,
        judge_contract::FUNCTION_ID,
        payload,
        JUDGE_TIMEOUT_MS + BUS_TIMEOUT_MS,
    )
    .await;
    let mut triage = asked.clone();
    let (code, cost_known) = match answered {
        Ok(EvaluateResponse::Ok {
            model,
            mut results,
            stats,
        }) => {
            triage.model = Some(model);
            triage.usage = Some(serde_json::to_value(&stats)?);
            let answer = results
                .remove(TRIAGE_EVALUATION)
                .and_then(|mut result| result.answers.remove(TRIAGE_QUESTION))
                .filter(|answer| judge_contract::validate_answer(&question, answer).is_ok());
            match answer {
                Some(answer) => {
                    triage.answer = Some(serde_json::to_value(&answer)?);
                    let choice = match &answer {
                        judge_contract::Answer::Choice { choice, .. } => choice.clone(),
                        _ => String::new(),
                    };
                    match choice.as_str() {
                        "relevant_opportunity" => {
                            triage.outcome = TriageOutcomeV1::Proceed;
                            triage.reason = "the judge found a relevant opportunity".into();
                            (None, stats.usage_complete)
                        }
                        "expected_behavior" => {
                            triage.outcome = TriageOutcomeV1::NoAction;
                            triage.reason = "the judge found expected behavior".into();
                            (Some(OutcomeCodeV1::NoAction), stats.usage_complete)
                        }
                        _ => {
                            triage.outcome = TriageOutcomeV1::Unknown;
                            triage.reason = "the judge found the evidence insufficient".into();
                            (Some(OutcomeCodeV1::Inconclusive), stats.usage_complete)
                        }
                    }
                }
                None => {
                    triage.reason = "the judge answered outside the question".into();
                    (Some(OutcomeCodeV1::Inconclusive), false)
                }
            }
        }
        Ok(EvaluateResponse::Error { code, stats, .. }) => {
            triage.usage = Some(serde_json::to_value(&stats)?);
            triage.reason = format!("the judge is unavailable: {code:?}");
            (Some(OutcomeCodeV1::Inconclusive), false)
        }
        Err(error) => {
            triage.reason = format!("the judge did not answer: {error}");
            (Some(OutcomeCodeV1::Inconclusive), false)
        }
    };
    // Judge usage is in tokens; its dollar cost is not reported.
    let _ = cost_known;
    settle(deps, &record.cycle_id, &work.work_id, None).await?;
    let current = state::get_cycle(&deps.iii, &record.cycle_id)
        .await?
        .ok_or_else(|| EvalError::NotFound(record.cycle_id.clone()))?;
    match code {
        None => {
            let detail = triage.reason.clone();
            transition(
                deps,
                &current.cycle_id,
                current.step,
                CycleStatusV1::Running,
                CycleStageV1::Baseline,
                &detail,
                |next| {
                    next.triage = Some(triage.clone());
                    Ok(())
                },
            )
            .await
        }
        Some(code) => {
            let detail = triage.reason.clone();
            transition(
                deps,
                &current.cycle_id,
                current.step,
                CycleStatusV1::Completed,
                CycleStageV1::Done,
                &detail,
                |next| {
                    next.triage = Some(triage.clone());
                    next.outcome = Some(CycleOutcomeV1 {
                        code,
                        detail: detail.clone(),
                    });
                    Ok(())
                },
            )
            .await
        }
    }
}

// ---------------------------------------------------------------------------
// Proposal
// ---------------------------------------------------------------------------

fn criterion_text(criterion: &CriterionV1) -> String {
    let target = match (&criterion.criterion_id, &criterion.pattern) {
        (Some(id), _) => format!(" {id}"),
        (_, Some(pattern)) => format!(" {pattern}"),
        _ => String::new(),
    };
    if criterion.min_effect > 0.0 {
        format!(
            "{:?}{target} of {} must {:?} by at least {} over {} run(s) a side",
            criterion.metric,
            criterion.scenario_id,
            criterion.direction,
            criterion.min_effect,
            criterion.min_runs
        )
    } else {
        format!(
            "{:?}{target} of {} must not regress",
            criterion.metric, criterion.scenario_id
        )
    }
}

/// The proposal input, all of it supplied by the cycle: the frozen
/// objective and criteria and the native evidence the analyses recorded.
fn proposal_input(record: &CycleRecordV1, plan: &CyclePlanV1) -> ProposalInputV1 {
    let mut evidence: Vec<CandidateEvidenceV1> = findings_of(record, true)
        .into_iter()
        .map(|(execution_id, finding)| CandidateEvidenceV1 {
            source: format!("e2e execution {execution_id}: {}", finding.signal),
            excerpt: finding.observation,
        })
        .collect();
    if let Some(baseline) = &record.baseline {
        for side in std::iter::once(&baseline.primary).chain(&baseline.preserved) {
            if let Some(mean) = side.mean {
                evidence.push(CandidateEvidenceV1 {
                    source: format!("e2e baseline {}", baseline.execution_id),
                    excerpt: format!("measured mean {mean} over {} run(s)", side.n),
                });
            }
        }
        evidence.push(CandidateEvidenceV1 {
            source: format!("e2e baseline {}", baseline.execution_id),
            excerpt: format!(
                "primary metric {:?} on {}: mean {:?} over {} run(s)",
                plan.criterion.metric,
                plan.criterion.scenario_id,
                baseline.primary.mean,
                baseline.primary.n
            ),
        });
    }
    evidence.truncate(MAX_EVIDENCE);
    let executions: Vec<String> = record
        .executions
        .iter()
        .filter_map(|entry| entry.execution_id.clone())
        .collect();
    ProposalInputV1 {
        case_ids: plan.sampling.seeds.clone(),
        objective: plan.objective.clone(),
        occurrence: format!(
            "cycle {} over E2E executions {}",
            record.cycle_id,
            executions.join(", ")
        ),
        evidence,
        criteria: std::iter::once(&plan.criterion)
            .chain(&plan.preserved)
            .map(criterion_text)
            .collect(),
    }
}

fn terminal(status: &CandidateStatusV1) -> bool {
    matches!(
        status,
        CandidateStatusV1::Completed | CandidateStatusV1::Failed { .. }
    )
}

async fn candidate_error(
    deps: &Deps,
    record: &CycleRecordV1,
    what: &str,
    error: CandidateError,
) -> Result<Option<CycleRecordV1>, EvalError> {
    match error {
        CandidateError::Inconclusive(reason) | CandidateError::UnsupportedIsolation(reason) => {
            stop(
                deps,
                record,
                OutcomeCodeV1::Inconclusive,
                format!("{what}: {reason}"),
            )
            .await
        }
        CandidateError::Dependency(reason) => {
            Err(EvalError::Dependency(format!("{what}: {reason}")))
        }
        other => fail(deps, record, &format!("{what}: {other}")).await,
    }
}

/// Waits at the current stage for the sweep to poll the actor again.
async fn wait_here(
    deps: &Deps,
    record: &CycleRecordV1,
    detail: String,
    mut mutate: impl FnMut(&mut CycleRecordV1),
) -> Result<Option<CycleRecordV1>, EvalError> {
    transition(
        deps,
        &record.cycle_id,
        record.step,
        CycleStatusV1::Waiting,
        record.stage,
        &detail,
        |next| {
            mutate(next);
            Ok(())
        },
    )
    .await
}

/// The bounded read-only proposal: reserved, its stable identity persisted,
/// then started (idempotently) and polled until it proposes a hypothesis
/// and change backed by the evidence, or stays inconclusive.
pub async fn proposal_stage(
    deps: &Deps,
    record: &CycleRecordV1,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let (Some(config), Some(plan)) = (candidate_config(record).cloned(), record.plan.clone())
    else {
        return stop(
            deps,
            record,
            OutcomeCodeV1::Inconclusive,
            "a proposal needs a frozen plan and config.candidate".into(),
        )
        .await;
    };
    if let Err(error) =
        candidate_config(record).map_or(Ok(()), |config| deps.check_candidate(config))
    {
        return stop(
            deps,
            record,
            OutcomeCodeV1::Inconclusive,
            format!("candidate authority: {error}"),
        )
        .await;
    }
    let input = proposal_input(record, &plan);
    let work_id = format!("{}:proposal", record.cycle_id);
    let Some(state_now) = record.proposal.clone() else {
        if input.evidence.is_empty() {
            return stop(
                deps,
                record,
                OutcomeCodeV1::Inconclusive,
                "no native evidence to propose a change from".into(),
            )
            .await;
        }
        let identity = match candidate::proposal_identity(&config, &record.cycle_id, &input) {
            Ok(identity) => identity,
            Err(error) => return candidate_error(deps, record, "proposal", error).await,
        };
        let work = CycleWorkV1 {
            work_id,
            kind: CycleWorkKindV1::Investigation,
            estimated_usd: candidate::estimate(&config).max_cost_usd,
            candidate: false,
        };
        if let cost::Reservation::OverLimit(detail) = reserve(deps, record, &work).await? {
            return end(deps, record, OutcomeCodeV1::Budget, &detail).await;
        }
        // The identity is persisted before the actor is started.
        return transition(
            deps,
            &record.cycle_id,
            record.step,
            CycleStatusV1::Running,
            CycleStageV1::Proposal,
            &format!("proposal {} reserved", identity.proposal_id),
            |next| {
                next.proposal = Some(ProposalStateV1 {
                    identity: identity.clone(),
                    proposal: None,
                });
                Ok(())
            },
        )
        .await;
    };
    let polled = match &state_now.proposal {
        Some(proposal) if !matches!(proposal.status, CandidateStatusV1::NotStarted { .. }) => {
            candidate::poll_proposal(&deps.iii, &config, proposal).await
        }
        _ => candidate::start_proposal(&deps.iii, &config, &record.cycle_id, &input).await,
    };
    let proposal = match polled {
        Ok(proposal) => proposal,
        Err(error) => return candidate_error(deps, record, "proposal", error).await,
    };
    let keep = |next: &mut CycleRecordV1, proposal: &crate::candidate::ProposalV1| {
        if let Some(state) = next.proposal.as_mut() {
            state.proposal = Some(proposal.clone());
        }
    };
    if !terminal(&proposal.status) {
        return wait_here(
            deps,
            record,
            format!("proposal {:?}", proposal.status),
            |next| keep(next, &proposal),
        )
        .await;
    }
    settle(deps, &record.cycle_id, &work_id, proposal.cost.cost_usd).await?;
    match candidate::proposal_brief(&proposal) {
        Ok(_) => {
            transition(
                deps,
                &record.cycle_id,
                record.step,
                CycleStatusV1::Running,
                CycleStageV1::Candidate,
                "an evidence-backed hypothesis and change were proposed",
                |next| {
                    keep(next, &proposal);
                    Ok(())
                },
            )
            .await
        }
        Err(error) => {
            let detail = format!("proposal: {error}");
            transition(
                deps,
                &record.cycle_id,
                record.step,
                CycleStatusV1::Completed,
                CycleStageV1::Done,
                &detail,
                |next| {
                    keep(next, &proposal);
                    next.outcome = Some(CycleOutcomeV1 {
                        code: OutcomeCodeV1::Inconclusive,
                        detail: detail.clone(),
                    });
                    Ok(())
                },
            )
            .await
        }
    }
}

// ---------------------------------------------------------------------------
// Candidate
// ---------------------------------------------------------------------------

fn git_tree(repository: &str, commit: &str) -> Result<String, EvalError> {
    let output = std::process::Command::new("git")
        .args(["-C", repository, "rev-parse", &format!("{commit}^{{tree}}")])
        .output()
        .map_err(|error| EvalError::Dependency(format!("git rev-parse: {error}")))?;
    let tree = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || tree.len() != 40 {
        return Err(EvalError::Dependency(format!(
            "git rev-parse {commit}^{{tree}} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(tree)
}

/// The isolated candidate actor on the proposal's brief: reserved as a
/// candidate, its identity persisted, started and polled; a completed commit
/// is read back (base, candidate sha, changed paths, tree) and published to
/// its own branch when it must be validated on a pinned stack.
pub async fn candidate_stage(
    deps: &Deps,
    record: &CycleRecordV1,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let Some(config) = candidate_config(record).cloned() else {
        return stop(
            deps,
            record,
            OutcomeCodeV1::Inconclusive,
            "config.candidate is absent".into(),
        )
        .await;
    };
    if let Err(error) =
        candidate_config(record).map_or(Ok(()), |config| deps.check_candidate(config))
    {
        return stop(
            deps,
            record,
            OutcomeCodeV1::Inconclusive,
            format!("candidate authority: {error}"),
        )
        .await;
    }
    let brief = match record
        .proposal
        .as_ref()
        .and_then(|state| state.proposal.as_ref())
        .map(candidate::proposal_brief)
    {
        Some(Ok(brief)) => brief,
        Some(Err(error)) => return candidate_error(deps, record, "candidate brief", error).await,
        None => return fail(deps, record, "a candidate without a proposal").await,
    };
    // Not `:candidate`: that is the candidate's validation E2E run.
    let work_id = format!("{}:implementation", record.cycle_id);
    let Some(state_now) = record.candidate.clone() else {
        let identity = match candidate::identity(&config, &record.cycle_id, &brief) {
            Ok(identity) => identity,
            Err(error) => return candidate_error(deps, record, "candidate", error).await,
        };
        let work = CycleWorkV1 {
            work_id,
            kind: CycleWorkKindV1::Implementation,
            estimated_usd: candidate::estimate(&config).max_cost_usd,
            candidate: true,
        };
        if let cost::Reservation::OverLimit(detail) = reserve(deps, record, &work).await? {
            return end(deps, record, OutcomeCodeV1::Budget, &detail).await;
        }
        return transition(
            deps,
            &record.cycle_id,
            record.step,
            CycleStatusV1::Running,
            CycleStageV1::Candidate,
            &format!(
                "candidate {} reserved on branch {}",
                identity.candidate_id, identity.branch
            ),
            |next| {
                next.candidate = Some(CandidateStateV1 {
                    identity: identity.clone(),
                    candidate: None,
                    candidate_tree: None,
                    publish: None,
                });
                Ok(())
            },
        )
        .await;
    };
    let polled = match &state_now.candidate {
        Some(current) if terminal(&current.status) => Ok(current.clone()),
        Some(current) if !matches!(current.status, CandidateStatusV1::NotStarted { .. }) => {
            candidate::poll(&deps.iii, &config, current).await
        }
        _ => candidate::start(&deps.iii, &config, &record.cycle_id, &brief).await,
    };
    let current = match polled {
        Ok(current) => current,
        Err(error) => return candidate_error(deps, record, "candidate", error).await,
    };
    let keep = |next: &mut CycleRecordV1, current: &crate::candidate::CandidateV1| {
        if let Some(state) = next.candidate.as_mut() {
            state.candidate = Some(current.clone());
        }
    };
    if !terminal(&current.status) {
        return wait_here(
            deps,
            record,
            format!("candidate {:?}", current.status),
            |next| keep(next, &current),
        )
        .await;
    }
    if state_now.candidate.as_ref().map(|stored| &stored.status) != Some(&current.status) {
        settle(deps, &record.cycle_id, &work_id, current.cost.cost_usd).await?;
    }
    let commit = match (&current.status, &current.commit) {
        (CandidateStatusV1::Completed, Some(commit)) => commit.clone(),
        (status, _) => {
            let detail = format!("the candidate produced no commit: {status:?}");
            return transition(
                deps,
                &record.cycle_id,
                record.step,
                CycleStatusV1::Completed,
                CycleStageV1::Done,
                &detail,
                |next| {
                    keep(next, &current);
                    next.outcome = Some(CycleOutcomeV1 {
                        code: OutcomeCodeV1::Inconclusive,
                        detail: detail.clone(),
                    });
                    Ok(())
                },
            )
            .await;
        }
    };
    let tree = match &state_now.candidate_tree {
        Some(tree) => tree.clone(),
        None => git_tree(&config.repository, &commit.candidate_sha)?,
    };
    let validates = record
        .config
        .as_ref()
        .is_some_and(|stored| stored.config.validation.is_some());
    if !validates {
        let detail = format!(
            "candidate {} changes {}; it cannot be validated: config.validation is absent",
            commit.candidate_sha,
            commit
                .paths
                .iter()
                .map(|path| path.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        return transition(
            deps,
            &record.cycle_id,
            record.step,
            CycleStatusV1::Completed,
            CycleStageV1::Done,
            &detail,
            |next| {
                keep(next, &current);
                if let Some(state) = next.candidate.as_mut() {
                    state.candidate_tree = Some(tree.clone());
                }
                next.outcome = Some(CycleOutcomeV1 {
                    code: OutcomeCodeV1::Inconclusive,
                    detail: detail.clone(),
                });
                Ok(())
            },
        )
        .await;
    }
    let publish = match &state_now.publish {
        Some(publish) => publish.clone(),
        // Bounded: a push that does not answer in time has an unknown remote
        // outcome and is retried; the deterministic branch and the
        // non-forced push make a retry reconcile, never duplicate.
        None => match candidate::publish(&config, &current, PUBLISH_TIMEOUT).await {
            Ok(publish) => publish,
            Err(CandidateError::InvalidConfig(reason)) => {
                return stop(
                    deps,
                    record,
                    OutcomeCodeV1::Inconclusive,
                    format!(
                        "candidate {} cannot be validated: {reason}",
                        commit.candidate_sha
                    ),
                )
                .await
            }
            Err(error) => {
                return candidate_error(deps, record, "candidate publication", error).await
            }
        },
    };
    transition(
        deps,
        &record.cycle_id,
        record.step,
        CycleStatusV1::Running,
        CycleStageV1::Validation,
        &format!(
            "candidate {} on {} changes {}",
            commit.candidate_sha,
            publish.branch,
            commit
                .paths
                .iter()
                .map(|path| path.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        |next| {
            keep(next, &current);
            if let Some(state) = next.candidate.as_mut() {
                state.candidate_tree = Some(tree.clone());
                state.publish = Some(publish.clone());
            }
            Ok(())
        },
    )
    .await
}

/// Dispatches the candidate's source validation under the frozen plan:
/// the same scenarios, cases, runs, subject and fixed stack, with only the
/// Harness pinned to the candidate commit.
pub async fn validation_stage(
    deps: &Deps,
    record: &CycleRecordV1,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let (Some(plan), Some(validation), Some(sha)) = (
        record.plan.clone(),
        record
            .config
            .as_ref()
            .and_then(|stored| stored.config.validation.clone()),
        candidate_sha(record),
    ) else {
        return fail(
            deps,
            record,
            "a validation needs a plan, config.validation and a candidate commit",
        )
        .await;
    };
    let parameters = ExecutionParametersV1 {
        where_: validation.where_,
        stack: Some(validation.source_stack(&sha)),
        ..plan.parameters.clone()
    };
    dispatch(
        deps,
        record,
        ExecutionRoleV1::Candidate,
        "candidate",
        &parameters,
        plan.estimated_usd,
    )
    .await
}

/// Stops the actors a cancelled cycle still runs (best effort).
pub async fn stop_actors(deps: &Deps, record: &CycleRecordV1) {
    let sessions = [
        record.proposal.as_ref().and_then(|state| {
            let proposal = state.proposal.as_ref();
            (!proposal.is_some_and(|proposal| terminal(&proposal.status))).then(|| {
                (
                    state.identity.session_id.clone(),
                    proposal.and_then(|proposal| proposal.turn_id.clone()),
                )
            })
        }),
        record.candidate.as_ref().and_then(|state| {
            let current = state.candidate.as_ref();
            (!current.is_some_and(|current| terminal(&current.status))).then(|| {
                (
                    state.identity.session_id.clone(),
                    current.and_then(|current| current.turn_id.clone()),
                )
            })
        }),
    ];
    for (session_id, turn_id) in sessions.into_iter().flatten() {
        if let Err(error) = call::<_, Value>(
            deps,
            "harness::stop",
            json!({ "session_id": session_id, "turn_id": turn_id }),
            BUS_TIMEOUT_MS,
        )
        .await
        {
            tracing::warn!(%session_id, %error, "could not stop a cancelled cycle's actor");
        }
    }
}
