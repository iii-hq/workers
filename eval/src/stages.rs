//! The stages between a measured baseline (or a triaged execution) and the
//! candidate's source validation: the semantic triage of ambiguous
//! findings, the bounded read-only proposal, the isolated candidate actor
//! and the dispatch of its validation. Each reserves its work before it
//! dispatches, persists the actor's stable identity before starting it, and
//! is polled by the sweep while it runs.

use std::collections::BTreeMap;

use judge_contract::{ErrorCode, EvaluateRequest, EvaluateResponse, Evaluation, Question};
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
use crate::{cost, ids, insight, replay, state};

const JUDGE_TIMEOUT_MS: u64 = 60_000;
/// The default LLM reads every site in one completion.
const ANALYST_TIMEOUT_MS: u64 = 180_000;
/// One replayed answer: a single generation of a recorded step.
const REPLAY_TIMEOUT_MS: u64 = 180_000;
/// Replayed answers in flight at once.
const REPLAY_CONCURRENCY: usize = 6;
const TRIAGE_EVALUATION: &str = "findings";
const TRIAGE_QUESTION: &str = "opportunity";
/// The least the judge's choice must lead the next option by before the
/// cycle acts on it: identical asks move JEV's probabilities by up to ~0.2.
const MIN_MARGIN: f64 = 0.2;
/// Native evidence entries handed to the proposal.
const MAX_EVIDENCE: usize = 20;
/// Characters of one baseline transcript handed to the proposal.
const MAX_TRANSCRIPT: usize = 6000;
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

/// What the cycle does with the judge's choice: it acts only on a clear lead.
fn verdict(answer: &judge_contract::Answer) -> (TriageOutcomeV1, Option<OutcomeCodeV1>, String) {
    let judge_contract::Answer::Choice {
        choice,
        probabilities,
        ..
    } = answer
    else {
        let reason = "the judge answered outside the question".into();
        return (
            TriageOutcomeV1::Unknown,
            Some(OutcomeCodeV1::Inconclusive),
            reason,
        );
    };
    let chosen = probabilities.get(choice).copied().unwrap_or(0.0);
    let next = probabilities
        .iter()
        .filter(|(key, _)| *key != choice)
        .fold(0.0, |max, (_, p)| f64::max(max, *p));
    let odds = format!("p {chosen:.2} against {next:.2}");
    if chosen - next < MIN_MARGIN {
        return (
            TriageOutcomeV1::Unknown,
            Some(OutcomeCodeV1::Inconclusive),
            format!("the judge could not tell ({choice}, {odds})"),
        );
    }
    match choice.as_str() {
        "relevant_opportunity" => (
            TriageOutcomeV1::Proceed,
            None,
            format!("the judge found a relevant opportunity ({odds})"),
        ),
        "expected_behavior" => (
            TriageOutcomeV1::NoAction,
            Some(OutcomeCodeV1::NoAction),
            format!("the judge found expected behavior ({odds})"),
        ),
        _ => (
            TriageOutcomeV1::Unknown,
            Some(OutcomeCodeV1::Inconclusive),
            format!("the judge found the evidence insufficient ({odds})"),
        ),
    }
}

/// The judge did not answer: say why and leave the cycle in triage. The
/// question has no side effect, so the sweep asks again until the cycle's
/// time budget ends it.
// ponytail: re-asked every 15 s sweep; back off if a long outage floods the judge.
async fn wait_for_judge(
    deps: &Deps,
    record: &CycleRecordV1,
    mut triage: TriageV1,
    why: String,
) -> Result<Option<CycleRecordV1>, EvalError> {
    triage.reason =
        format!("the judge did not answer ({why}); it is asked again on the next sweep");
    state::update_cycle(&deps.iii, &record.cycle_id, |stored| {
        if stored.step != record.step || stored.status.is_terminal() {
            return Ok((None, ()));
        }
        let mut next = stored.clone();
        next.triage = Some(triage.clone());
        Ok((Some(next), ()))
    })
    .await?;
    Ok(None)
}

/// Decides whether a source execution's pertinent findings go on to
/// candidate work. A clear deterministic defect needs no judgment; Sentinel's triage of the same occurrences is reused;
/// only an ambiguous improvement opportunity is asked of `judge::evaluate`,
/// with what each scenario expects. The question has no side effect, so a
/// restart or an unavailable judge asks it again until the cycle's time
/// budget runs out.
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
    match reserve(deps, record, &work).await? {
        cost::Reservation::OverLimit(detail) => {
            return end(deps, record, OutcomeCodeV1::Budget, &detail).await;
        }
        // Asked before (a restart, or a judge that did not answer): ask
        // again until the cycle's time budget runs out.
        cost::Reservation::Existing(_) => {
            if let Some(detail) = state::get_ledger(&deps.iii, &record.cycle_id)
                .await?
                .and_then(|ledger| cost::exceeded(&ledger, ids::now_ms()))
            {
                let detail = format!("the judge never answered: {detail}");
                return end(deps, record, OutcomeCodeV1::Budget, &detail).await;
            }
        }
        cost::Reservation::Reserved(_) => {}
    }
    // What each scenario expects and what each lost criterion asks: without
    // them the judge calls infrastructure noise an opportunity.
    let mut scenarios: Vec<&ScenarioContextV1> = record
        .analyses
        .iter()
        .flat_map(|analysis| &analysis.scenarios)
        .collect();
    scenarios.dedup_by_key(|scenario| &scenario.scenario_id);
    let state_value = json!({
        "objective": "Does this group of findings show a relevant Harness improvement opportunity?",
        "scenarios": scenarios,
        "findings": pertinent.iter().map(|(execution_id, finding)| json!({
            "execution_id": execution_id, "class": finding.class, "signal": finding.signal,
            "observation": finding.observation, "criterion_asks": finding.asks})).collect::<Vec<_>>(),
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
    // Persisted before the call, so a restart asks again under the same id.
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
    let code = match answered {
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
                    let code;
                    (triage.outcome, code, triage.reason) = verdict(&answer);
                    code
                }
                None => {
                    triage.reason = "the judge answered outside the question".into();
                    Some(OutcomeCodeV1::Inconclusive)
                }
            }
        }
        Ok(EvaluateResponse::Error { code, stats, .. })
            if matches!(code, ErrorCode::InvalidRequest | ErrorCode::PayloadTooLarge) =>
        {
            triage.usage = Some(serde_json::to_value(&stats)?);
            triage.reason = format!("the judge refused the question: {code:?}");
            Some(OutcomeCodeV1::Inconclusive)
        }
        Ok(EvaluateResponse::Error { code, .. }) => {
            return wait_for_judge(deps, record, triage, format!("{code:?}")).await;
        }
        Err(error) => return wait_for_judge(deps, record, triage, error.to_string()).await,
    };
    // Judge usage is in tokens; its dollar cost is not reported.
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

/// Reads the measured runs' transcripts before any proposal: each errored
/// call is a site Jev judges (whose doing it is); when Jev is not
/// configured, does not answer or decides no site, the default LLM reads the
/// sites. The preventable patterns become improvement points; none ends the cycle
/// `no_action` before any paid proposal.
pub async fn insight_stage(
    deps: &Deps,
    record: &CycleRecordV1,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let Some(plan) = record.plan.clone() else {
        return fail(deps, record, "an insight needs a frozen plan").await;
    };
    let key = record
        .executions
        .iter()
        .find(|entry| {
            entry.role == ExecutionRoleV1::Baseline && entry.state == ExecutionStateV1::Imported
        })
        .and_then(|entry| entry.admission_key.clone());
    let Some(evidence) = (match key {
        Some(key) => state::get_evidence(&deps.iii, &key).await?,
        None => None,
    }) else {
        return fail(deps, record, "an insight needs the imported baseline").await;
    };
    let mut sites = insight::sites(
        &evidence.execution_id,
        &evidence.bundle,
        &plan.parameters.scenarios,
    );
    let mut found = InsightV1 {
        sites: Vec::new(),
        points: Vec::new(),
        source: None,
        reason: String::new(),
        request_id: None,
        model: None,
        usage: None,
    };
    let close = |found: InsightV1, next: Result<CycleStageV1, OutcomeCodeV1>| async move {
        let detail = found.reason.clone();
        let (status, stage) = match next {
            Ok(stage) => (CycleStatusV1::Running, stage),
            Err(_) => (CycleStatusV1::Completed, CycleStageV1::Done),
        };
        transition(
            deps,
            &record.cycle_id,
            record.step,
            status,
            stage,
            &detail,
            |next_record| {
                next_record.insight = Some(found.clone());
                if let Err(code) = next {
                    next_record.outcome = Some(CycleOutcomeV1 {
                        code,
                        detail: detail.clone(),
                    });
                }
                Ok(())
            },
        )
        .await
    };
    if plan.criterion.metric == CriterionMetricV1::Tokens {
        (found.points, found.reason) = insight::spend(&evidence.bundle, &plan.parameters.scenarios);
        found.source = Some(InsightSourceV1::Transcripts);
        // The objective names the spend itself: no point stops nothing.
        let next = after_points(record, &plan, &mut found, false);
        return close(found, next).await;
    }
    if sites.is_empty() {
        found.reason = "the measured runs made no failed function call to read".into();
        return close(found, Err(OutcomeCodeV1::NoAction)).await;
    }
    let summaries: BTreeMap<&str, &str> = record
        .analyses
        .iter()
        .flat_map(|analysis| &analysis.scenarios)
        .filter_map(|scenario| Some((scenario.scenario_id.as_str(), scenario.summary.as_deref()?)))
        .collect();
    // Jev first: one choice per site.
    let mut no_jev = None;
    match record
        .config
        .as_ref()
        .and_then(|stored| stored.config.judge.clone())
    {
        None => no_jev = Some("no judge is configured (config.judge)".to_string()),
        Some(judge) => {
            let work = CycleWorkV1 {
                work_id: format!("{}:insight", record.cycle_id),
                kind: CycleWorkKindV1::Jev,
                estimated_usd: judge.estimated_usd,
                candidate: false,
            };
            if let cost::Reservation::OverLimit(detail) = reserve(deps, record, &work).await? {
                return end(deps, record, OutcomeCodeV1::Budget, &detail).await;
            }
            let question = insight::question();
            // One evaluation per failure pattern; its verdict holds for every
            // call in it.
            let patterns: Vec<(String, Vec<String>, Value)> = insight::patterns(&sites)
                .iter()
                .map(|pattern| {
                    (
                        pattern.id.clone(),
                        pattern
                            .sites
                            .iter()
                            .map(|site| site.site_id.clone())
                            .collect(),
                        insight::pattern_state(
                            &plan.objective,
                            |scenario| summaries.get(scenario).map(|summary| summary.to_string()),
                            pattern,
                        ),
                    )
                })
                .collect();
            let request_id = format!("{}-insight", record.cycle_id);
            found.request_id = Some(request_id.clone());
            let request = EvaluateRequest {
                options: Default::default(),
                request_id: Some(request_id),
                model: None,
                timeout_ms: JUDGE_TIMEOUT_MS,
                expires_at_unix_ms: None,
                evaluations: patterns
                    .iter()
                    .map(|(id, _, state)| Evaluation {
                        id: id.clone(),
                        state: state.clone(),
                        questions: BTreeMap::from([(insight::QUESTION.into(), question.clone())]),
                    })
                    .collect(),
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
            settle(deps, &record.cycle_id, &work.work_id, None).await?;
            match answered {
                Ok(EvaluateResponse::Ok {
                    model,
                    mut results,
                    stats,
                }) => {
                    found.model = Some(model);
                    found.usage = Some(serde_json::to_value(&stats)?);
                    for (id, members, _) in &patterns {
                        let answer = results
                            .remove(id)
                            .and_then(|mut result| result.answers.remove(insight::QUESTION))
                            .filter(|answer| {
                                judge_contract::validate_answer(&question, answer).is_ok()
                            });
                        if let Some(answer) = answer {
                            let (class, probability) = insight::verdict(&answer);
                            for site in sites
                                .iter_mut()
                                .filter(|site| members.contains(&site.site_id))
                            {
                                site.class = Some(class);
                                site.probability = Some(probability);
                            }
                        }
                    }
                    if sites.iter().any(|site| {
                        site.class
                            .is_some_and(|class| class != SiteClassV1::Unclear)
                    }) {
                        found.source = Some(InsightSourceV1::Jev);
                        found.points = insight::points(&sites);
                        found.reason = insight::tally(&sites);
                    } else {
                        no_jev = Some(format!("{}: no clear lead on any", insight::tally(&sites)));
                    }
                }
                Ok(EvaluateResponse::Error { code, .. }) => {
                    no_jev = Some(format!("Jev did not answer ({code:?})"));
                }
                Err(error) => no_jev = Some(format!("Jev did not answer ({error})")),
            }
        }
    }
    // The default LLM reads what Jev could not tell: every failed call when
    // Jev did not answer or decided none, else the ones it left unclear.
    let unread: Vec<ErrorSiteV1> = match &no_jev {
        Some(_) => sites.clone(),
        None => sites
            .iter()
            .filter(|site| site.class.is_none_or(|class| class == SiteClassV1::Unclear))
            .cloned()
            .collect(),
    };
    if !unread.is_empty() {
        let work = CycleWorkV1 {
            work_id: format!("{}:analyst", record.cycle_id),
            kind: CycleWorkKindV1::Investigation,
            estimated_usd: insight::ANALYST_ESTIMATED_USD,
            candidate: false,
        };
        if let cost::Reservation::OverLimit(detail) = reserve(deps, record, &work).await? {
            return end(deps, record, OutcomeCodeV1::Budget, &detail).await;
        }
        let (system, user) = insight::analyst_prompt(&plan.objective, &unread);
        // The configured actors' model reads too (a subscription costs
        // nothing per call); else the default.
        let (provider, model) = candidate_config(record).map_or(
            (insight::ANALYST_PROVIDER, insight::ANALYST_MODEL),
            |config| (config.provider.as_str(), config.model.as_str()),
        );
        let answered: Result<Value, EvalError> = call(
            deps,
            "router::complete",
            json!({"model": model, "provider": provider,
                "system_prompt": system, "max_output_tokens": insight::ANALYST_MAX_OUTPUT,
                "messages": [{"role": "user", "timestamp": ids::now_ms(),
                    "content": [{"type": "text", "text": user}]}]}),
            ANALYST_TIMEOUT_MS,
        )
        .await;
        let cost = answered.as_ref().ok().and_then(billed);
        settle(deps, &record.cycle_id, &work.work_id, cost).await?;
        let read = answered
            .map_err(|error| error.to_string())
            .and_then(|reply| {
                let text: String = reply["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|part| part["text"].as_str())
                    .collect();
                insight::parse_points(&text, &unread)
            });
        let read_by = format!("the default LLM ({model})");
        match (read, &no_jev) {
            (Ok(points), Some(why)) => {
                found.source = Some(InsightSourceV1::Llm);
                found.model = Some(model.into());
                found.reason = format!(
                    "{why}; {read_by} read the {} failed call(s) and found {} point(s)",
                    unread.len(),
                    points.len()
                );
                found.points = points;
            }
            (Ok(points), None) => {
                found.reason = format!(
                    "{}; {read_by} read the {} unclear and found {} point(s)",
                    found.reason,
                    unread.len(),
                    points.len()
                );
                found.points.extend(points);
            }
            (Err(error), Some(why)) => {
                found.sites = sites;
                found.reason =
                    format!("{why}, and the default LLM could not read the transcripts: {error}");
                return close(found, Err(OutcomeCodeV1::Inconclusive)).await;
            }
            // Jev's verdicts stand; only the unclear stay unread.
            (Err(error), None) => {
                found.reason = format!(
                    "{}; {read_by} could not read the {} unclear: {error}",
                    found.reason,
                    unread.len()
                );
            }
        }
    }
    found.sites = sites;
    if found.points.is_empty() {
        found.reason = format!(
            "{}; none is a failure a Harness change could prevent",
            found.reason
        );
    }
    let next = after_points(record, &plan, &mut found, true);
    close(found, next).await
}

/// Where a cycle goes once its points stand: without any (when they are
/// `required`), room to prove a change or candidate work, nowhere; else to
/// the proposal.
fn after_points(
    record: &CycleRecordV1,
    plan: &CyclePlanV1,
    found: &mut InsightV1,
    required: bool,
) -> Result<CycleStageV1, OutcomeCodeV1> {
    if required && found.points.is_empty() {
        return Err(OutcomeCodeV1::NoAction);
    }
    if let Some(reason) = record.baseline.as_ref().and_then(|baseline| {
        crate::decision::no_room(
            &plan.criterion,
            plan.sampling.runs_per_side,
            &baseline.primary,
        )
    }) {
        found.reason = format!("{}; {reason}", found.reason);
        return Err(OutcomeCodeV1::NoAction);
    }
    if candidate_config(record).is_none() {
        found.reason = format!(
            "{}; candidate work is not configured (config.candidate), so no change is tried",
            found.reason
        );
        return Err(OutcomeCodeV1::Inconclusive);
    }
    Ok(CycleStageV1::Proposal)
}

/// The proposal input, all of it supplied by the cycle: the frozen
/// objective and criteria and the native evidence the analyses recorded,
/// led by what the agent did in a baseline run that lost the goal
/// (`baseline`: its imported bundle).
fn proposal_input(
    record: &CycleRecordV1,
    plan: &CyclePlanV1,
    baseline: Option<&serde_json::Value>,
) -> ProposalInputV1 {
    let transcript = baseline.and_then(|bundle| {
        crate::evidence::transcript_excerpt(
            bundle,
            &plan.criterion.scenario_id,
            plan.criterion.criterion_id.as_deref(),
            MAX_TRANSCRIPT,
        )
    });
    // What the insight read in the transcripts leads: each point with the
    // failed calls that show it.
    let mut evidence: Vec<CandidateEvidenceV1> = record
        .insight
        .iter()
        .flat_map(|insight| {
            insight.points.iter().map(|point| CandidateEvidenceV1 {
                source: format!(
                    "improvement point ({:?}) on {}: {}",
                    point.source, point.function_id, point.summary
                ),
                excerpt: point
                    .sites
                    .is_empty()
                    .then(|| point.summary.clone())
                    .into_iter()
                    .chain(
                        insight
                            .sites
                            .iter()
                            .filter(|site| point.sites.contains(&site.site_id))
                            .map(|site| {
                                format!(
                            "run {} entry {}: called {} with {}\nerror: {}\nbefore: {}\nnext: {}",
                            site.run_id,
                            site.entry_id,
                            site.function_id,
                            site.arguments,
                            site.error,
                            site.before,
                            site.after
                        )
                            }),
                    )
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            })
        })
        .collect();
    // What the insight read across every run: the model, the cache, the window.
    evidence.extend(record.insight.iter().map(|insight| CandidateEvidenceV1 {
        source: format!(
            "insight over the transcripts of {}",
            plan.criterion.scenario_id
        ),
        excerpt: insight.reason.clone(),
    }));
    evidence.extend(transcript.map(|(run_id, text)| CandidateEvidenceV1 {
        source: format!(
            "e2e baseline run {run_id} of {}: its transcript, one line per message",
            plan.criterion.scenario_id
        ),
        excerpt: text,
    }));
    evidence.extend(
        findings_of(record, true)
            .into_iter()
            .map(|(execution_id, finding)| CandidateEvidenceV1 {
                source: format!("e2e execution {execution_id}: {}", finding.signal),
                excerpt: finding.observation,
            }),
    );
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
            "cycle {} over E2E executions {}, run by {}/{}",
            record.cycle_id,
            executions.join(", "),
            plan.parameters.provider,
            plan.parameters.model
        ),
        evidence,
        criteria: std::iter::once(&plan.criterion)
            .chain(&plan.preserved)
            .map(|criterion| {
                // What the scenario asks of it, as its spec words it.
                let asks = plan
                    .tests
                    .iter()
                    .filter(|test| test.scenario_id == criterion.scenario_id)
                    .flat_map(|test| &test.criteria)
                    .find(|spec| Some(&spec.id) == criterion.criterion_id.as_ref())
                    .map(|spec| format!(" (asks: {})", spec.description))
                    .unwrap_or_default();
                format!("{}{asks}", criterion_text(criterion))
            })
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
    let baseline = match record.executions.iter().find(|entry| {
        entry.role == ExecutionRoleV1::Baseline && entry.state == ExecutionStateV1::Imported
    }) {
        Some(entry) => match &entry.admission_key {
            Some(key) => state::get_evidence(&deps.iii, key)
                .await?
                .map(|evidence| evidence.bundle),
            None => None,
        },
        None => None,
    };
    let input = proposal_input(record, &plan, baseline.as_ref());
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
    // What the insight pointed at is replayed first (level 1).
    let next_stage = match record.insight {
        Some(_) => CycleStageV1::Replay,
        None => CycleStageV1::Validation,
    };
    transition(
        deps,
        &record.cycle_id,
        record.step,
        CycleStatusV1::Running,
        next_stage,
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
    let Some(base_sha) = record
        .config
        .as_ref()
        .and_then(|stored| stored.config.candidate.as_ref())
        .map(|candidate| candidate.base_sha.clone())
    else {
        return fail(deps, record, "a validation needs config.candidate.base_sha").await;
    };
    // Nothing paid is run for a change no proof run could measure.
    let proving = record.executions.iter().any(|entry| {
        matches!(
            entry.role,
            ExecutionRoleV1::Candidate | ExecutionRoleV1::Control
        )
    });
    if !proving {
        if let Some(gap) = crate::decision::proof_gap(record) {
            return end(deps, record, OutcomeCodeV1::Inconclusive, &gap).await;
        }
    }
    let runs = plan.sampling.runs_per_side;
    let batched = validation.where_ == WhereV1::Github;
    let Some((role, name)) = proof_schedule(runs, batched)
        .into_iter()
        .find(|(_, name)| !proof_imported(record, name))
    else {
        return fail(
            deps,
            record,
            "every proof round is imported but nothing decided",
        )
        .await;
    };
    let parameters = |role: ExecutionRoleV1| ExecutionParametersV1 {
        where_: validation.where_,
        stack: Some(
            validation.source_stack(if role == ExecutionRoleV1::Control {
                &base_sha
            } else {
                &sha
            }),
        ),
        runs: if batched { runs } else { 1 },
        ..plan.parameters.clone()
    };
    let estimated = plan.estimated_usd / f64::from(if batched { 1 } else { runs.max(1) });
    let first = dispatch(deps, record, role, &name, &parameters(role), estimated).await?;
    // On GitHub the other side starts at once: a provider's bad spell then
    // falls on both sides alike, as ABBA spreads it over Docker's rounds.
    if batched {
        if let Some(next) = &first {
            let started = |name: &str| {
                let work_id = format!("{}:{name}", next.cycle_id);
                next.executions
                    .iter()
                    .any(|entry| entry.work_id.as_deref() == Some(work_id.as_str()))
            };
            if let Some((other, other_name)) = proof_schedule(runs, true)
                .into_iter()
                .find(|(_, other_name)| !started(other_name))
            {
                return dispatch(
                    deps,
                    next,
                    other,
                    &other_name,
                    &parameters(other),
                    estimated,
                )
                .await;
            }
        }
    }
    Ok(first)
}

/// The validation's executions in dispatch order. `batched` (GitHub, where
/// an execution runs its rounds as parallel jobs): one candidate execution
/// of every round and one of the base commit (`control`), started together.
/// Otherwise one round of every planned case per execution, the base and
/// the candidate alternating as ABBA (candidate first in odd rounds), so
/// drift over time and a warmed provider cache fall on both sides alike.
pub fn proof_schedule(runs: u32, batched: bool) -> Vec<(ExecutionRoleV1, String)> {
    if batched {
        return vec![
            (ExecutionRoleV1::Candidate, "candidate-1".into()),
            (ExecutionRoleV1::Control, "control-1".into()),
        ];
    }
    (1..=runs)
        .flat_map(|round| {
            let candidate = (ExecutionRoleV1::Candidate, format!("candidate-{round}"));
            let control = (ExecutionRoleV1::Control, format!("control-{round}"));
            if round % 2 == 1 {
                [candidate, control]
            } else {
                [control, candidate]
            }
        })
        .collect()
}

/// Whether the proof execution `name` is imported.
pub fn proof_imported(record: &CycleRecordV1, name: &str) -> bool {
    let work_id = format!("{}:{name}", record.cycle_id);
    record.executions.iter().any(|entry| {
        entry.work_id.as_deref() == Some(work_id.as_str())
            && entry.state == ExecutionStateV1::Imported
    })
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

/// Level 1 (`crate::replay`): the base's failed steps rebuilt by the base
/// and the candidate build and sampled on both, before the runs. The runs
/// follow whatever it finds; it decides the counted criterion only when it
/// shows a change.
pub async fn replay_stage(
    deps: &Deps,
    record: &CycleRecordV1,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let measured = Box::pin(replay_steps(deps, record)).await?;
    transition(
        deps,
        &record.cycle_id,
        record.step,
        CycleStatusV1::Running,
        CycleStageV1::Validation,
        &measured.reason.clone(),
        |next| {
            next.replay = Some(measured.clone());
            Ok(())
        },
    )
    .await
}

fn unmeasured(reason: impl Into<String>) -> ReplayV1 {
    ReplayV1 {
        outcome: ValidationOutcomeV1::Inconclusive,
        reason: reason.into(),
        points: Vec::new(),
        base_samples: 0,
        base_mistakes: 0,
        candidate_samples: 0,
        candidate_mistakes: 0,
        p_fewer: None,
        p_more: None,
        steps: None,
        base_tokens: None,
        candidate_tokens: None,
    }
}

async fn replay_steps(deps: &Deps, record: &CycleRecordV1) -> Result<ReplayV1, EvalError> {
    let commit = record
        .candidate
        .as_ref()
        .and_then(|state| state.candidate.as_ref())
        .and_then(|candidate| candidate.commit.as_ref());
    let (Some(config), Some(commit), Some(insight)) =
        (candidate_config(record), commit, record.insight.as_ref())
    else {
        return Ok(unmeasured("no candidate commit or failed calls to replay"));
    };
    let key = record
        .executions
        .iter()
        .find(|entry| {
            entry.role == ExecutionRoleV1::Baseline && entry.state == ExecutionStateV1::Imported
        })
        .and_then(|entry| entry.admission_key.clone());
    let Some(evidence) = (match key {
        Some(key) => state::get_evidence(&deps.iii, &key).await?,
        None => None,
    }) else {
        return Ok(unmeasured("the baseline's transcripts are not stored"));
    };
    if let Some(plan) = record
        .plan
        .as_ref()
        .filter(|plan| plan.criterion.metric == CriterionMetricV1::Tokens)
    {
        return Box::pin(replay_sizes(plan, config, commit, &evidence.bundle)).await;
    }
    let mut points = replay::points(&evidence.bundle, &insight.sites);
    let mut schemas = BTreeMap::new();
    fetch_schemas(
        deps,
        points
            .iter()
            .flat_map(|point| replay::targets(&point.cause)),
        &mut schemas,
    )
    .await;
    // Only a mistake the replay can see again: a call refused before it ran.
    points.retain(|point| replay::mistake(&point.cause, &schemas).is_some());
    if points.is_empty() {
        return Ok(unmeasured(
            "no failed call was refused for its arguments, the only mistake a replay judges",
        ));
    }

    let (base_bin, candidate_bin) = match build_pair(config, commit).await? {
        Ok(bins) => bins,
        Err(reason) => return Ok(unmeasured(reason)),
    };

    // Each step's request under both builds; a step the candidate leaves
    // alone cannot tell the builds apart and is not sampled.
    let mut rebuilt = Vec::new();
    let mut measured: Vec<ReplayPointV1> = Vec::new();
    let planned_model = record_model(record);
    for point in &points {
        if rebuilt.len() == replay::MAX_POINTS {
            break;
        }
        let model = point.cause["model"].as_str().unwrap_or(&planned_model);
        let provider = point.cause["provider"].as_str().unwrap_or_default();
        let input = replay::input(point, model, provider);
        let (base, candidate) = match (
            harness_replay(&base_bin, &input).await,
            harness_replay(&candidate_bin, &input).await,
        ) {
            (Ok(base), Ok(candidate)) => (base, candidate),
            (Err(error), _) | (_, Err(error)) => {
                return Ok(unmeasured(format!(
                    "step {} of run {} could not be rebuilt: {error}",
                    point.step, point.run_id
                )))
            }
        };
        let changed = base != candidate;
        measured.push(ReplayPointV1 {
            site_id: point.site_id.clone(),
            run_id: point.run_id.clone(),
            step: point.step,
            after: point.after,
            changed,
            exact: replay::exact(&base["req"], &point.recorded),
            samples: 0,
            base_mistakes: 0,
            candidate_mistakes: 0,
        });
        if changed {
            rebuilt.push((measured.len() - 1, request(base), request(candidate)));
        }
    }
    if rebuilt.is_empty() {
        let mut found = unmeasured(
            "the candidate sends the failed steps the same request as the base, so a replay \
             cannot tell them apart",
        );
        found.points = measured;
        return Ok(found);
    }

    let per_step = replay::per_step(rebuilt.len());
    let work = CycleWorkV1 {
        work_id: format!("{}:replay", record.cycle_id),
        kind: CycleWorkKindV1::Investigation,
        estimated_usd: f64::from(per_step)
            * rebuilt.len() as f64
            * 2.0
            * replay::ESTIMATED_USD_PER_SAMPLE,
        candidate: false,
    };
    if let cost::Reservation::OverLimit(detail) = reserve(deps, record, &work).await? {
        return Ok(unmeasured(format!("the replay is over budget: {detail}")));
    }
    let jobs: Vec<(usize, bool, Value)> = rebuilt
        .iter()
        .flat_map(|(at, base, candidate)| {
            (0..per_step)
                .flat_map(move |_| [(*at, false, base.clone()), (*at, true, candidate.clone())])
        })
        .collect();
    use futures_util::StreamExt as _;
    let answers: Vec<(usize, bool, Value)> = futures_util::stream::iter(jobs)
        .map(|(at, is_candidate, request)| async move {
            let answer = call::<_, Value>(deps, "router::complete", request, REPLAY_TIMEOUT_MS)
                .await
                .ok();
            (at, is_candidate, answer)
        })
        .buffer_unordered(REPLAY_CONCURRENCY)
        .filter_map(|(at, is_candidate, answer)| async move {
            answer.map(|answer| (at, is_candidate, answer))
        })
        .collect()
        .await;
    let cost: f64 = answers
        .iter()
        .filter_map(|(_, _, answer)| billed(answer))
        .sum();
    settle(deps, &record.cycle_id, &work.work_id, Some(cost)).await?;

    fetch_schemas(
        deps,
        answers
            .iter()
            .flat_map(|(_, _, answer)| replay::targets(&answer["message"])),
        &mut schemas,
    )
    .await;
    let (mut base, mut candidate) = ((0, 0), (0, 0));
    for (at, is_candidate, answer) in &answers {
        let wrong = u32::from(replay::mistake(&answer["message"], &schemas).is_some());
        let point = &mut measured[*at];
        point.samples = per_step;
        let side = if *is_candidate {
            point.candidate_mistakes += wrong;
            &mut candidate
        } else {
            point.base_mistakes += wrong;
            &mut base
        };
        side.0 += wrong;
        side.1 += 1;
    }
    let (outcome, reason) = replay::verdict(base, candidate);
    let (p_fewer, p_more) = replay::fisher(base, candidate);
    Ok(ReplayV1 {
        outcome,
        reason,
        points: measured,
        base_samples: base.1,
        base_mistakes: base.0,
        candidate_samples: candidate.1,
        candidate_mistakes: candidate.0,
        p_fewer: Some(p_fewer),
        p_more: Some(p_more),
        ..unmeasured("")
    })
}

/// What a `router::complete` answer cost: its reported cost, or nothing
/// when it reports usage without one (a subscription bills no call).
fn billed(reply: &Value) -> Option<f64> {
    let usage = reply.get("usage")?;
    Some(usage["cost_usd"].as_f64().unwrap_or(0.0))
}

/// `harness-replay` of the base and of the candidate commit.
async fn build_pair(
    config: &CandidateConfigV1,
    commit: &candidate::CandidateCommitV1,
) -> Result<Result<(std::path::PathBuf, std::path::PathBuf), String>, EvalError> {
    let (config, base_sha, candidate_sha) = (
        config.clone(),
        commit.base_sha.clone(),
        commit.candidate_sha.clone(),
    );
    tokio::task::spawn_blocking(move || {
        let built = candidate::replay_binary(&config, &base_sha)
            .and_then(|base| Ok((base, candidate::replay_binary(&config, &candidate_sha)?)));
        candidate::prune_replay_binaries(&config, &[&base_sha, &candidate_sha]);
        built.map_err(|error| format!("the replay could not be built: {error}"))
    })
    .await
    .map_err(|error| EvalError::Dependency(format!("replay build: {error}")))
}

/// A token objective's level 1: every recorded step of the baseline runs
/// rebuilt by both builds (`harness-replay` sizes), at the recorded
/// trajectory. Deterministic: nothing is sampled. Characters become tokens
/// at the rate the base rebuild matches the recorded input.
async fn replay_sizes(
    plan: &CyclePlanV1,
    config: &CandidateConfigV1,
    commit: &candidate::CandidateCommitV1,
    bundle: &Value,
) -> Result<ReplayV1, EvalError> {
    let (base_bin, candidate_bin) = match build_pair(config, commit).await? {
        Ok(bins) => bins,
        Err(reason) => return Ok(unmeasured(reason)),
    };
    let (mut base, mut candidate, mut recorded) = (0.0, 0.0, 0.0);
    let (mut steps, mut changed, mut exact, mut runs) = (0u32, 0u32, 0u32, 0u32);
    for (scenario, run) in crate::evidence::runs(bundle) {
        if !plan
            .parameters
            .scenarios
            .iter()
            .any(|wanted| scenario["scenario_id"] == wanted.as_str())
        {
            continue;
        }
        let Some(entries) = run["transcript"]["messages"].as_array() else {
            continue;
        };
        let Some(answer) = entries
            .iter()
            .map(|entry| &entry["message"])
            .find(|message| message["role"] == "assistant")
        else {
            continue;
        };
        let input = json!({"sizes": {"entries": entries,
            "model": answer["model"].as_str().unwrap_or(&plan.parameters.model),
            "provider": answer["provider"]}});
        let (rebuilt_base, rebuilt_candidate) = match (
            harness_replay(&base_bin, &input).await,
            harness_replay(&candidate_bin, &input).await,
        ) {
            (Ok(b), Ok(c)) => (b, c),
            (Err(error), _) | (_, Err(error)) => {
                return Ok(unmeasured(format!(
                    "run {} could not be rebuilt: {error}",
                    run["run_id"].as_str().unwrap_or_default()
                )))
            }
        };
        let recorded_of: BTreeMap<&str, &Value> = entries
            .iter()
            .filter_map(|entry| Some((entry["entry_id"].as_str()?, entry)))
            .collect();
        for (b, c) in rebuilt_base
            .as_array()
            .into_iter()
            .flatten()
            .zip(rebuilt_candidate.as_array().into_iter().flatten())
        {
            let Some(entry) = b["entry_id"].as_str().and_then(|id| recorded_of.get(id)) else {
                continue;
            };
            // In the metric's units: uncached input, against what a prefix
            // cache cannot serve. A change that breaks the cache pays more.
            recorded += entry["message"]["usage"]["input"].as_f64().unwrap_or(0.0);
            base += b["fresh"].as_f64().unwrap_or(0.0);
            candidate += c["fresh"].as_f64().unwrap_or(0.0);
            steps += 1;
            changed += u32::from(b != c);
            let matched = replay::exact(&b["req"], &entry["origin"]["req"]);
            exact += u32::from(matched.iter().any(|part| part == "messages"));
        }
        runs += 1;
    }
    if steps == 0 || base <= 0.0 {
        return Ok(unmeasured("no recorded step to rebuild"));
    }
    let per_run = recorded / base / f64::from(runs);
    let (base_tokens, candidate_tokens) = (base * per_run, candidate * per_run);
    let (outcome, reason) = replay::size_verdict(
        base_tokens,
        candidate_tokens,
        plan.criterion.min_effect,
        steps,
        changed,
    );
    Ok(ReplayV1 {
        outcome,
        reason: format!("{reason} ({exact} of {steps} step(s) rebuilt exactly as recorded)"),
        steps: Some(steps),
        base_tokens: Some(base_tokens),
        candidate_tokens: Some(candidate_tokens),
        ..unmeasured("")
    })
}

fn record_model(record: &CycleRecordV1) -> String {
    record
        .plan
        .as_ref()
        .map(|plan| plan.parameters.model.clone())
        .unwrap_or_default()
}

/// A rebuilt request as `router::complete` takes it.
fn request(mut rebuilt: Value) -> Value {
    if let Some(object) = rebuilt.as_object_mut() {
        object.remove("req");
    }
    rebuilt
}

/// Runs one `harness-replay` build on `input`.
async fn harness_replay(binary: &std::path::Path, input: &Value) -> Result<Value, String> {
    use tokio::io::AsyncWriteExt as _;
    let mut child = tokio::process::Command::new(binary)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec(input).map_err(|error| error.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&bytes)
            .await
            .map_err(|error| error.to_string())?;
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())
}

/// Adds the request schemas of `targets` not in `into` yet, as the registry
/// reports them; one it does not know stays out.
async fn fetch_schemas(
    deps: &Deps,
    targets: impl Iterator<Item = String>,
    into: &mut BTreeMap<String, Value>,
) {
    let wanted: std::collections::BTreeSet<String> =
        targets.filter(|id| !into.contains_key(id)).collect();
    if wanted.is_empty() {
        return;
    }
    let Ok(info) = call::<_, Value>(
        deps,
        "engine::functions::info",
        json!({ "function_ids": wanted }),
        BUS_TIMEOUT_MS,
    )
    .await
    else {
        return;
    };
    for function in info["functions"].as_array().into_iter().flatten() {
        if let (Some(id), Some(schema)) = (
            function["function_id"].as_str(),
            function
                .get("request_schema")
                .filter(|schema| schema.is_object()),
        ) {
            into.insert(id.to_string(), schema.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_github_proof_is_one_execution_a_side_and_a_docker_one_alternates() {
        let names = |runs, batched| -> Vec<String> {
            proof_schedule(runs, batched)
                .into_iter()
                .map(|(_, name)| name)
                .collect()
        };
        assert_eq!(names(17, true), ["candidate-1", "control-1"]);
        assert_eq!(
            names(2, false),
            ["candidate-1", "control-1", "control-2", "candidate-2"]
        );
    }

    fn choice(
        choice: &str,
        relevant: f64,
        expected: f64,
        insufficient: f64,
    ) -> judge_contract::Answer {
        judge_contract::Answer::Choice {
            choice: choice.into(),
            probabilities: BTreeMap::from([
                ("relevant_opportunity".into(), relevant),
                ("expected_behavior".into(), expected),
                ("insufficient_evidence".into(), insufficient),
            ]),
            confidence: 0.0,
        }
    }

    // Probabilities JEV answered for real and control findings (24/10).
    #[test]
    fn the_cycle_acts_only_on_a_clear_lead() {
        let (outcome, code, _) = verdict(&choice("relevant_opportunity", 0.86, 0.06, 0.08));
        assert_eq!((outcome, code), (TriageOutcomeV1::Proceed, None));
        let (outcome, code, reason) = verdict(&choice("relevant_opportunity", 0.45, 0.36, 0.19));
        assert_eq!(
            (outcome, code),
            (TriageOutcomeV1::Unknown, Some(OutcomeCodeV1::Inconclusive))
        );
        assert!(reason.contains("could not tell"), "{reason}");
        let (outcome, code, _) = verdict(&choice("expected_behavior", 0.1, 0.8, 0.1));
        assert_eq!(
            (outcome, code),
            (TriageOutcomeV1::NoAction, Some(OutcomeCodeV1::NoAction))
        );
        let (_, code, _) = verdict(&choice("insufficient_evidence", 0.22, 0.14, 0.64));
        assert_eq!(code, Some(OutcomeCodeV1::Inconclusive));
    }
}
