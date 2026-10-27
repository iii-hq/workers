//! The E2E's native reports, read in code: execution status, the executable
//! identity, per-run evidence and the deterministic first pass. Everything
//! reads the `e2e::dashboard::execution-get` bundle
//! (`{manifest: {executions: [summary]}, detail: {reports: [{scenario_id,
//! available, state?, report: {system_under_test, subject, scenarios: [..]}}]}}`)
//! as copied into the loop, never the live E2E.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::contract::*;
use crate::{diagnostics, ids};

/// Summary statuses of an execution that finished normally, passed or not
/// (`technical_failed` is the infrastructure's, still analyzable).
pub const TERMINAL_STATUSES: [&str; 4] = ["passed", "failed", "incomplete", "technical_failed"];
/// Statuses of an execution still in progress.
pub const OPEN_STATUSES: [&str; 6] = [
    "queued",
    "pending",
    "starting",
    "running",
    "cancelling",
    "importing",
];
/// Report states of a slot that has not reported yet.
const OPEN_REPORT_STATES: [&str; 3] = ["running", "queued", "admitting"];

pub fn text_at(value: &Value, pointer: &str) -> Option<String> {
    match value.pointer(pointer)? {
        Value::String(text) => Some(text.trim().to_string()).filter(|text| !text.is_empty()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// The execution's own summary in a bundle.
pub fn summary<'a>(bundle: &'a Value, execution_id: &str) -> Option<&'a Value> {
    bundle
        .pointer("/manifest/executions")?
        .as_array()?
        .iter()
        .find(|entry| entry["id"] == execution_id)
}

/// An immutable dispatch key identifies work only when the complete listing
/// has one match.
pub(crate) fn unique_execution_id(listed: &Value, idempotency_key: &str) -> Option<String> {
    if !listed["next_cursor"].is_null() {
        return None;
    }
    let mut matches = listed["executions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["idempotency_key"] == idempotency_key)
        .filter_map(|entry| entry["id"].as_str().map(str::to_string));
    let execution_id = matches.next();
    execution_id.filter(|_| matches.next().is_none())
}

pub fn status_of(summary: &Value) -> String {
    text_at(summary, "/status")
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Where an `execution-get` bundle stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// Terminal and every report slot has reported (or failed to).
    Complete,
    /// Still running, or terminal with slots that have not reported yet.
    Open(String),
    /// Ended without a normal result (cancelled, unknown status).
    Abnormal(String),
}

pub fn readiness(bundle: &Value, execution_id: &str) -> Readiness {
    let Some(summary) = summary(bundle, execution_id) else {
        return Readiness::Abnormal("execution-get does not list the execution".into());
    };
    let status = status_of(summary);
    if OPEN_STATUSES.contains(&status.as_str()) {
        return Readiness::Open(format!("status {status}"));
    }
    if !TERMINAL_STATUSES.contains(&status.as_str()) {
        return Readiness::Abnormal(format!("status {status:?} is not a normal result"));
    }
    let open = reports(bundle)
        .filter(|report| {
            text_at(report, "/state")
                .is_some_and(|state| OPEN_REPORT_STATES.contains(&state.as_str()))
        })
        .count();
    if open > 0 {
        return Readiness::Open(format!("{open} report slot(s) still running or queued"));
    }
    Readiness::Complete
}

fn reports(bundle: &Value) -> impl Iterator<Item = &Value> {
    bundle["detail"]["reports"].as_array().into_iter().flatten()
}

fn native_reports(bundle: &Value) -> impl Iterator<Item = &Value> {
    reports(bundle)
        .map(|report| &report["report"])
        .filter(|report| report.is_object())
}

fn scenarios(bundle: &Value) -> impl Iterator<Item = &Value> {
    native_reports(bundle).flat_map(|report| report["scenarios"].as_array().into_iter().flatten())
}

/// The Harness commit a GitHub campaign built: it records it only in its
/// resolved stack (`harness: "@<sha7>"`), beside the built binary's digest,
/// not as `harness_source`. A short revision, matched as a prefix.
fn github_built_revision(system: &Value) -> Option<String> {
    let short = system["stack"]["stack_versions"]["harness"]
        .as_str()?
        .strip_prefix('@')?;
    let hex = |text: &str| text.bytes().all(|byte| byte.is_ascii_hexdigit());
    (short.len() >= 7
        && hex(short)
        && system["harness_binary_sha256"]
            .as_str()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .is_some_and(|digest| digest.len() == 64 && hex(digest)))
    .then(|| short.to_string())
}

/// The one value every report agrees on, or `None` (absent or disagreeing).
fn agreed(values: impl Iterator<Item = Option<String>>) -> Option<String> {
    let mut seen = BTreeSet::new();
    for value in values {
        seen.insert(value?);
    }
    (seen.len() == 1).then(|| seen.into_iter().next().unwrap_or_default())
}

/// The E2E's own digest of a JSON value (`artifact::sha256_value`): the value
/// with every object's keys sorted, compact `serde_json` bytes, SHA-256 as
/// `sha256:<hex>`. It reproduces the catalog's `contract_sha256` from a
/// native scenario's `case` and `execution_policy`.
pub fn native_sha256(value: &Value) -> String {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
            Value::Object(map) => {
                let mut entries: Vec<(&String, &Value)> = map.iter().collect();
                entries.sort_by(|left, right| left.0.cmp(right.0));
                Value::Object(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key.clone(), canonical(value)))
                        .collect(),
                )
            }
            other => other.clone(),
        }
    }
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(&canonical(value)).unwrap_or_default();
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// The catalog's `contract_sha256` of a native scenario, recomputed the way
/// the E2E computes it (`scenario_contract_sha256`: `{scenario_id, case,
/// execution_policy}` with the case's own scenario id). `None` when the
/// report does not carry the whole case or its policy.
pub fn scenario_contract_sha256(scenario: &Value) -> Option<String> {
    let case = scenario.get("case").filter(|case| case.is_object())?;
    let policy = scenario
        .get("execution_policy")
        .filter(|policy| policy.is_object())?;
    let scenario_id = case.get("scenario_id").and_then(Value::as_str)?;
    for field in ["case_id", "behavior_sha256", "inputs_sha256"] {
        text_at(case, &format!("/{field}"))?;
    }
    case.get("seed").and_then(Value::as_u64)?;
    case.get("deliverable_contract")?;
    Some(native_sha256(&serde_json::json!({
        "scenario_id": scenario_id,
        "case": case,
        "execution_policy": policy,
    })))
}

/// The digest of a native `report.subject` (`ModelArtifact`: model,
/// provider, the agent profile's id and configuration digest when an agent
/// ran, context and output limits, tool/vision support), only when the
/// object is complete.
fn subject_sha256(subject: &Value) -> Option<String> {
    let text = |key: &str| {
        subject
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    };
    let complete = subject.is_object()
        && text("model")
        && text("provider")
        && subject
            .get("context_window")
            .and_then(Value::as_u64)
            .is_some()
        && subject
            .get("max_output_tokens")
            .and_then(Value::as_u64)
            .is_some()
        && ["supports_tools", "supports_vision"].iter().all(|key| {
            subject
                .get(*key)
                .is_some_and(|value| value.is_null() || value.is_boolean())
        })
        && match subject.get("agent") {
            None | Some(Value::Null) => true,
            Some(agent) => ["id", "configuration_sha256"].iter().all(|key| {
                agent
                    .get(*key)
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
            }),
        };
    complete.then(|| native_sha256(subject))
}

/// The worker that is the product under test in a Registry stack identity.
pub const HARNESS_STACK_WORKER: &str = "harness";

/// The digest of every stack component but the Harness, from a native
/// `system_under_test.stack` in Registry mode (`stack_versions`, worker →
/// version, as the engine reported it installed; a worker built from source
/// reports `@<sha7>`, the Harness included). Only the `harness` entry is
/// removed; it must be present. A Source-mode stack names one workers
/// revision and proves no other runtime identity: unknown, never guessed.
fn fixed_stack_sha256(stack: &Value) -> Option<String> {
    if stack.get("mode").and_then(Value::as_str) != Some("registry") {
        return None;
    }
    let versions = stack.get("stack_versions")?.as_object()?;
    let valid = versions.iter().all(|(worker, version)| {
        !worker.trim().is_empty()
            && version
                .as_str()
                .is_some_and(|version| !version.trim().is_empty())
    });
    if !valid || !versions.contains_key(HARNESS_STACK_WORKER) {
        return None;
    }
    let mut fixed = versions.clone();
    fixed.remove(HARNESS_STACK_WORKER);
    // A worker built from a commit (`@<sha>`) is what each side builds from
    // its own commit: the stack proof checks it against the template.
    fixed.retain(|_, version| {
        !version
            .as_str()
            .is_some_and(|version| version.starts_with('@'))
    });
    (!fixed.is_empty()).then(|| native_sha256(&Value::Object(fixed)))
}

pub fn identity(bundle: &Value, execution_id: &str) -> EvidenceIdentityV1 {
    let parameters = summary(bundle, execution_id)
        .map(|summary| summary["parameters"].clone())
        .unwrap_or(Value::Null);
    let system =
        |pointer: &str| agreed(native_reports(bundle).map(|report| text_at(report, pointer)));
    let digest = |values: Vec<&Value>| {
        (!values.is_empty() && values.iter().all(|value| !value.is_null()))
            .then(|| ids::sha256_json(&values))
    };
    let mut scenario_ids = Vec::new();
    for scenario in scenarios(bundle) {
        // The scenario's own fields must agree with the case it ran.
        let consistent = |pointer: &str, in_case: &str| {
            let value = text_at(scenario, pointer);
            match text_at(scenario, in_case) {
                Some(case) if value.as_deref() != Some(case.as_str()) => None,
                _ => value,
            }
        };
        let identity = ScenarioIdentityV1 {
            scenario_id: text_at(scenario, "/scenario_id").unwrap_or_default(),
            case_id: consistent("/case_id", "/case/case_id"),
            behavior_sha256: consistent("/behavior_sha256", "/case/behavior_sha256"),
            inputs_sha256: text_at(scenario, "/case/inputs_sha256"),
            seed: scenario.pointer("/case/seed").and_then(Value::as_u64),
            contract_sha256: scenario_contract_sha256(scenario),
        };
        if !scenario_ids.contains(&identity) {
            scenario_ids.push(identity);
        }
    }
    scenario_ids.sort_by(|left, right| left.scenario_id.cmp(&right.scenario_id));
    let natives: Vec<&Value> = native_reports(bundle).collect();
    EvidenceIdentityV1 {
        scenarios: scenario_ids,
        contract_digest: digest(
            natives
                .iter()
                .map(|report| &report["system_under_test"]["contract_hashes"])
                .chain(
                    natives
                        .iter()
                        .map(|report| &report["result_contract_sha256"]),
                )
                .collect(),
        ),
        artifacts_digest: bundle
            .pointer("/detail/reports")
            .filter(|reports| reports.as_array().is_some_and(|items| !items.is_empty()))
            .map(ids::sha256_json),
        stack_digest: digest(
            natives
                .iter()
                .map(|report| &report["system_under_test"]["stack"])
                .collect(),
        ),
        subject_sha256: agreed(
            native_reports(bundle).map(|report| subject_sha256(&report["subject"])),
        ),
        fixed_stack_digest: agreed(
            native_reports(bundle)
                .map(|report| fixed_stack_sha256(&report["system_under_test"]["stack"])),
        ),
        model: system("/subject/model").or_else(|| text_at(&parameters, "/model")),
        provider: system("/subject/provider").or_else(|| text_at(&parameters, "/provider")),
        harness_version: system("/system_under_test/harness_version"),
        // SHA-256 of the running Harness executable the E2E captured: every
        // report (and the manifest, when it states one) must agree.
        harness_binary_sha256: agreed(
            native_reports(bundle)
                .map(|report| text_at(report, "/system_under_test/harness_binary_sha256"))
                .chain(
                    bundle
                        .pointer("/manifest/system_under_test/harness_binary_sha256")
                        .map(|_| {
                            text_at(bundle, "/manifest/system_under_test/harness_binary_sha256")
                        }),
                ),
        ),
        workers_revision: agreed(natives.iter().map(|report| {
            let source = &report["system_under_test"]["harness_source"];
            if source.is_null() {
                return github_built_revision(&report["system_under_test"]);
            }
            let revision = source["revision"].as_str()?;
            let binary = source["binary_sha256"].as_str()?.strip_prefix("sha256:")?;
            (source["repository"] == "iii-hq/workers"
                && source["path"] == "harness"
                && revision.len() == 40
                && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
                && binary.len() == 64
                && binary
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                && source["binary_sha256"] == report["system_under_test"]["harness_binary_sha256"])
                .then(|| revision.to_string())
        })),
        engine_version: system("/system_under_test/engine_version"),
        e2e_revision: system("/system_under_test/e2e_revision"),
    }
}

/// Every native run with its scenario.
pub fn runs(bundle: &Value) -> Vec<(&Value, &Value)> {
    scenarios(bundle)
        .flat_map(|scenario| {
            scenario["runs"]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |run| (scenario, run))
        })
        .collect()
}

/// `awarded / possible` per evaluated criterion of a native run
/// (`criteria: [{id, possible, awarded: null|number, gate, reason}]`).
fn criterion_scores(run: &Value) -> BTreeMap<String, f64> {
    run["criteria"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|criterion| {
            let possible = criterion["possible"]
                .as_f64()
                .filter(|value| *value > 0.0)?;
            let awarded = criterion["awarded"].as_f64()?;
            Some((criterion["id"].as_str()?.to_string(), awarded / possible))
        })
        .collect()
}

/// The earlier attempts the E2E retried within a run's slot.
fn retry_attempts(run: &Value) -> &[Value] {
    run["retry_attempts"].as_array().map_or(&[], Vec::as_slice)
}

/// The transcript of the first run of `scenario_id` that lost `criterion`
/// (else its first run), one line per message: who spoke, what it called,
/// the start of each text. At most `max` characters, cut at a line; `None`
/// when no run kept a transcript. What an actor reads to see what the agent
/// did, not only how it scored.
pub fn transcript_excerpt(
    bundle: &Value,
    scenario_id: &str,
    criterion: Option<&str>,
    max: usize,
) -> Option<(String, String)> {
    let runs: Vec<&Value> = runs(bundle)
        .into_iter()
        .filter(|(scenario, run)| {
            scenario["scenario_id"] == scenario_id
                && run["transcript"]["messages"]
                    .as_array()
                    .is_some_and(|messages| !messages.is_empty())
        })
        .map(|(_, run)| run)
        .collect();
    let lost = |run: &&&Value| {
        criterion.is_some_and(|id| {
            criterion_scores(run)
                .get(id)
                .is_some_and(|score| *score < 1.0)
        })
    };
    let run = runs.iter().find(lost).or(runs.first())?;
    let clip = |text: &str, at: usize| {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        match text.char_indices().nth(at) {
            Some((cut, _)) => format!("{}…", &text[..cut]),
            None => text,
        }
    };
    let mut lines = Vec::new();
    for entry in run["transcript"]["messages"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let Some(message) = entry.get("message") else {
            let kind = entry["custom"]["custom_type"].as_str().unwrap_or("entry");
            lines.push(format!("(notice: {kind})"));
            continue;
        };
        let role = message["role"].as_str().unwrap_or("?");
        let parts: Vec<String> = message["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|part| match part["type"].as_str() {
                Some("text") => Some(clip(part["text"].as_str()?, 240)),
                Some("function_call") => {
                    let arguments = &part["arguments"];
                    let target = arguments["function"]
                        .as_str()
                        .or(part["function_id"].as_str())
                        .unwrap_or("?");
                    Some(format!(
                        "calls {target} {}",
                        clip(&arguments.to_string(), 160)
                    ))
                }
                Some(other) => Some(format!("({other})")),
                None => None,
            })
            .collect();
        lines.push(format!("{role}: {}", parts.join(" | ")));
    }
    let mut text = String::new();
    for line in lines {
        if text.len() + line.len() + 1 > max {
            text.push('…');
            break;
        }
        text.push_str(&line);
        text.push('\n');
    }
    Some((text_at(run, "/run_id").unwrap_or_default(), text))
}

/// Every run of `scenario_id` as evidence, with `patterns` counted by the
/// detectors on its own transcript.
pub fn runs_of(bundle: &Value, scenario_id: &str, patterns: &[String]) -> Vec<EvidenceRunV1> {
    runs(bundle)
        .into_iter()
        .filter(|(scenario, _)| scenario["scenario_id"] == scenario_id)
        .map(|(scenario, run)| {
            let (completion, technical) = (text_at(run, "/completion"), text_at(run, "/technical"));
            let signals = run["transcript"]["messages"].as_array().map(|messages| {
                let mut counts: BTreeMap<String, u32> = patterns
                    .iter()
                    .map(|pattern| (pattern.clone(), 0))
                    .collect();
                let session = run["session_id"].as_str().unwrap_or_default();
                for diagnostic in diagnostics::detect(session, messages).diagnostics {
                    if let Some(count) = counts.get_mut(&diagnostic.pattern()) {
                        *count += 1;
                    }
                }
                counts
            });
            let valid = technical
                .as_deref()
                .is_some_and(|technical| technical.eq_ignore_ascii_case("valid"));
            // The E2E retries only an attempt that failed: a slot that needed
            // one did not complete, however the retry ended.
            let retried = !retry_attempts(run).is_empty();
            EvidenceRunV1 {
                run_id: text_at(run, "/run_id").unwrap_or_default(),
                case_id: text_at(scenario, "/case_id"),
                completed: valid && completion.as_deref() == Some("completed") && !retried,
                completion,
                technical,
                status: text_at(run, "/status"),
                cost_usd: run["cost"]["total_usd"].as_f64(),
                wall_time_ms: run["wall_time_ms"].as_f64(),
                total_tokens: run["efficiency"]["total_tokens"].as_f64(),
                function_calls: run["efficiency"]["function_calls"].as_f64(),
                // Read from the transcript, not `efficiency.function_call_errors`:
                // that total is `harness::metrics`, which a candidate may change.
                function_call_errors: run["transcript"]["messages"].as_array().map(|messages| {
                    messages
                        .iter()
                        .filter(|entry| {
                            diagnostics::role(entry) == "function_result"
                                && entry["message"]["is_error"] == true
                        })
                        .count() as f64
                }),
                criteria: criterion_scores(run),
                signals,
            }
        })
        .collect()
}

/// The execution's reported cost: the sum of every run's `cost.total_usd`,
/// or `None` (unknown) when any run did not report one.
pub fn reported_cost(bundle: &Value) -> Option<f64> {
    let runs = runs(bundle);
    if runs.is_empty() {
        return None;
    }
    runs.iter()
        .try_fold(0.0, |total, (_, run)| {
            let cost = run["cost"]["total_usd"].as_f64()?;
            (cost.is_finite() && cost >= 0.0).then_some(total + cost)
        })
        .filter(|total| total.is_finite())
}

/// The canonical cases the execution's runs reported.
pub fn seeds(bundle: &Value) -> Vec<String> {
    let mut seeds: Vec<String> = scenarios(bundle)
        .filter_map(|scenario| text_at(scenario, "/case_id"))
        .collect();
    seeds.sort();
    seeds.dedup();
    seeds
}

fn finding(
    key: &str,
    class: FindingClassV1,
    signal: &str,
    subject: &str,
    observation: String,
) -> FindingV1 {
    FindingV1 {
        fingerprint: ids::sha256_json(&(key, signal, subject, class)),
        class,
        signal: signal.into(),
        observation,
        asks: None,
    }
}

/// What each reported scenario expects and how many runs it had.
pub fn scenario_context(bundle: &Value) -> Vec<ScenarioContextV1> {
    scenarios(bundle)
        .map(|scenario| ScenarioContextV1 {
            scenario_id: text_at(scenario, "/scenario_id").unwrap_or_default(),
            title: text_at(scenario, "/title"),
            summary: text_at(scenario, "/summary"),
            runs: scenario["runs"]
                .as_array()
                .map_or(0, |runs| runs.len() as u32),
        })
        .collect()
}

/// The E2E's `FailureDomain`s (`runs[].failures[].domain`).
pub const NATIVE_FAILURE_DOMAINS: [&str; 3] = ["subject", "resource", "e2e_infrastructure"];

/// What one native run's outcome says, before its criteria.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RunOutcome {
    Held,
    Finding(FindingClassV1, String),
    Unknown(String),
}

fn failure_codes(failures: &[&Value], domain: &str) -> String {
    let codes: BTreeSet<String> = failures
        .iter()
        .filter(|failure| {
            text_at(failure, "/domain").is_some_and(|value| value.eq_ignore_ascii_case(domain))
        })
        .map(|failure| {
            text_at(failure, "/code")
                .or_else(|| text_at(failure, "/status"))
                .unwrap_or_else(|| "uncoded".into())
        })
        .collect();
    codes.into_iter().collect::<Vec<_>>().join(", ")
}

/// A run's outcome by the E2E's own failure domain first, its technical
/// badge second. A subject failure stays the subject's even on a
/// `technical_invalid` run; an explicit E2E infrastructure failure is the
/// infrastructure's; an exhausted resource or subject budget is a resource
/// finding that needs semantic or mechanism evidence before it means
/// anything about the Harness, never a defect by itself. Without a recorded
/// failure, the status names the domain (`subject_error`, `resource_limit`,
/// `infrastructure_error`), then the technical badge decides.
fn run_outcome(
    subject: &str,
    domains: &BTreeSet<String>,
    failures: &[&Value],
    technical: Option<&str>,
    status: Option<&str>,
    completion: Option<&str>,
) -> RunOutcome {
    use FindingClassV1::*;
    let domain = ["subject", "e2e_infrastructure", "resource"]
        .into_iter()
        .find(|domain| domains.contains(*domain))
        .or(match status {
            Some("subject_error") => Some("subject"),
            Some("infrastructure_error") => Some("e2e_infrastructure"),
            Some("resource_limit") => Some("resource"),
            _ => None,
        });
    let recorded = |domain: &str| {
        let codes = failure_codes(failures, domain);
        if codes.is_empty() {
            format!("status {}", status.unwrap_or("unreported"))
        } else {
            format!("recorded failure(s) {codes}")
        }
    };
    match domain {
        Some("subject") => RunOutcome::Finding(
            Defect,
            format!(
                "run {subject}: subject failure ({}), technical outcome {}",
                recorded("subject"),
                technical.unwrap_or("unreported")
            ),
        ),
        Some("e2e_infrastructure") => RunOutcome::Finding(
            InfrastructureOrEvaluatorFailure,
            format!("run {subject}: E2E infrastructure failure ({})", recorded("e2e_infrastructure")),
        ),
        Some(_) => RunOutcome::Finding(
            InsufficientEvidence,
            format!(
                "run {subject}: resource or subject budget exhausted ({}); a resource finding, not a \
                 defect until semantic or mechanism evidence shows why",
                recorded("resource")
            ),
        ),
        None => match (technical, status) {
            (None, _) => RunOutcome::Unknown(format!("technical outcome of run {subject}: not reported")),
            (Some(technical), _) if technical != "valid" => RunOutcome::Finding(
                InfrastructureOrEvaluatorFailure,
                format!("run {subject}: technical outcome {technical}"),
            ),
            (_, Some(status @ ("hard_gate_failed" | "failed"))) => RunOutcome::Finding(
                Defect,
                format!("run {subject} ended {status} with a valid technical outcome"),
            ),
            (_, Some("passed")) if completion != Some("completed") => RunOutcome::Finding(
                Defect,
                format!(
                    "run {subject} is `passed` but its completion is {}",
                    completion.unwrap_or("unreported")
                ),
            ),
            (_, Some("passed")) => RunOutcome::Held,
            (_, other) => RunOutcome::Unknown(format!(
                "status of run {subject}: {}",
                other.unwrap_or("not reported")
            )),
        },
    }
}

/// The deterministic first pass over the stored copy: no model call, no
/// new campaign. Returns the findings and what the copy could not tell.
/// `expected_behavior` is reached only when every run was fully assessed and
/// held; anything unknown is `insufficient_evidence`, never expected.
pub fn first_pass(evidence: &ExecutionEvidenceV1) -> (Vec<FindingV1>, Vec<String>) {
    let (bundle, key) = (&evidence.bundle, evidence.admission_key.as_str());
    let mut findings = Vec::new();
    let mut unknowns = BTreeSet::new();
    let identity = &evidence.identity;
    for (field, missing) in [
        ("contract digest", identity.contract_digest.is_none()),
        ("stack digest", identity.stack_digest.is_none()),
        ("subject digest", identity.subject_sha256.is_none()),
        (
            "fixed stack digest (Registry stack_versions without the Harness)",
            identity.fixed_stack_digest.is_none(),
        ),
        ("model", identity.model.is_none()),
        ("provider", identity.provider.is_none()),
        ("harness version", identity.harness_version.is_none()),
        ("e2e revision", identity.e2e_revision.is_none()),
    ] {
        if missing {
            unknowns.insert(format!(
                "{field}: not stated (or not agreed) by the reports"
            ));
        }
    }
    for report in reports(bundle).filter(|report| !report["report"].is_object()) {
        unknowns.insert(format!(
            "report of {}: unavailable ({})",
            text_at(report, "/scenario_id").unwrap_or_default(),
            text_at(report, "/error").unwrap_or_else(|| "no reason given".into())
        ));
    }
    let runs = runs(bundle);
    if runs.is_empty() {
        unknowns.insert("runs: no run in the imported reports".into());
    }
    let (mut fully_held, mut unreadable) = (true, 0);
    for (scenario, run) in &runs {
        let scenario_id = text_at(scenario, "/scenario_id").unwrap_or_default();
        let run_id = text_at(run, "/run_id").unwrap_or_default();
        let subject = format!("{scenario_id}/{run_id}");
        match run["transcript"]["messages"].as_array() {
            Some(messages) => {
                let session_id = run["session_id"].as_str().unwrap_or(&run_id);
                let detection = diagnostics::detect(session_id, messages);
                unreadable += detection.unreadable_messages;
                for diagnostic in detection.diagnostics {
                    let class = if diagnostic.rule_id == diagnostics::REPEATED_TOOL_ERROR {
                        FindingClassV1::Defect
                    } else {
                        FindingClassV1::ImprovementOpportunity
                    };
                    findings.push(FindingV1 {
                        fingerprint: diagnostic.fingerprint,
                        class,
                        signal: diagnostic.rule_id,
                        observation: diagnostic.observation,
                        asks: None,
                    });
                }
            }
            None => {
                unknowns.insert(format!("transcript of run {subject}: not preserved"));
            }
        }
        let technical = text_at(run, "/technical").map(|value| value.to_ascii_lowercase());
        let status = text_at(run, "/status").map(|value| value.to_ascii_lowercase());
        let completion = text_at(run, "/completion").map(|value| value.to_ascii_lowercase());
        let failures: Vec<&Value> = run["failures"].as_array().into_iter().flatten().collect();
        let domains: BTreeSet<String> = failures
            .iter()
            .filter_map(|failure| {
                text_at(failure, "/domain").map(|domain| domain.to_ascii_lowercase())
            })
            .collect();
        for domain in &domains {
            if !NATIVE_FAILURE_DOMAINS.contains(&domain.as_str()) {
                unknowns.insert(format!(
                    "failure domain {domain:?} of run {subject}: not a known E2E domain"
                ));
            }
        }
        let outcome = run_outcome(
            &subject,
            &domains,
            &failures,
            technical.as_deref(),
            status.as_deref(),
            completion.as_deref(),
        );
        let outcome = match outcome {
            RunOutcome::Unknown(reason) => {
                unknowns.insert(reason);
                fully_held = false;
                None
            }
            RunOutcome::Held => None,
            RunOutcome::Finding(class, observation) => Some((class, observation)),
        };
        let measurable = technical.as_deref() == Some("valid");
        if let Some((class, observation)) = outcome {
            fully_held = false;
            findings.push(finding(key, class, "run_outcome", &subject, observation));
        }
        let criteria = run["criteria"].as_array().cloned().unwrap_or_default();
        // A technically valid run whose criteria were not evaluated is the
        // evaluator's gap, never the Harness's; a broken run evaluated
        // nothing and says nothing.
        if criteria.is_empty() {
            fully_held = false;
            if measurable {
                findings.push(finding(
                    key,
                    FindingClassV1::EvaluationGap,
                    "criteria_not_evaluated",
                    &subject,
                    format!("run {subject}: technically valid, but no criterion was evaluated"),
                ));
            } else {
                unknowns.insert(format!(
                    "criteria of run {subject}: not assessed in the report"
                ));
            }
        }
        for criterion in &criteria {
            let id = text_at(criterion, "/id").unwrap_or_default();
            match (
                criterion["awarded"].as_f64(),
                criterion["possible"].as_f64(),
            ) {
                (Some(awarded), Some(possible)) if awarded < possible => {
                    fully_held = false;
                    findings.push(FindingV1 {
                        asks: text_at(criterion, "/description"),
                        ..finding(
                            key,
                            FindingClassV1::ImprovementOpportunity,
                            "criterion",
                            &format!("{subject}/{id}"),
                            format!(
                                "run {subject}: criterion {id} awarded {awarded} of {possible}: {}",
                                text_at(criterion, "/reason").unwrap_or_default()
                            ),
                        )
                    });
                }
                (Some(_), Some(_)) => {}
                _ if measurable => {
                    fully_held = false;
                    findings.push(finding(
                        key,
                        FindingClassV1::EvaluationGap,
                        "criterion_not_evaluated",
                        &format!("{subject}/{id}"),
                        format!(
                            "run {subject}: criterion {id} was not evaluated on a technically valid run: {}",
                            text_at(criterion, "/reason").unwrap_or_else(|| "no reason given".into())
                        ),
                    ));
                }
                _ => {
                    fully_held = false;
                    unknowns.insert(format!("criterion {id} of run {subject}: not evaluated"));
                }
            }
        }
    }
    if unreadable > 0 {
        unknowns.insert(format!(
            "{unreadable} assistant entr(y/ies) could not be interpreted by the detectors"
        ));
    }
    unknowns.insert(
        "child transcripts: the reports do not establish that every child transcript was kept"
            .into(),
    );
    let summary = summary(bundle, &evidence.execution_id)
        .cloned()
        .unwrap_or(Value::Null);
    let status = status_of(&summary);
    let runs_explain = findings
        .iter()
        .any(|finding| finding.signal == "run_outcome");
    let execution = match status.as_str() {
        "technical_failed" if !runs_explain => Some((
            FindingClassV1::InfrastructureOrEvaluatorFailure,
            "execution reported a technical failure".to_string(),
        )),
        "failed" if !runs_explain => Some((
            FindingClassV1::InfrastructureOrEvaluatorFailure,
            format!(
                "execution failed before its runs explained it: {}",
                text_at(&summary, "/first_failure/message").unwrap_or_else(|| "no message".into())
            ),
        )),
        _ if !findings.is_empty() => None,
        _ if fully_held && !runs.is_empty() => Some((
            FindingClassV1::ExpectedBehavior,
            "every run passed, completed and was awarded every criterion in full".to_string(),
        )),
        _ => Some((
            FindingClassV1::InsufficientEvidence,
            "not every run was fully assessed; nothing can be concluded about it".to_string(),
        )),
    };
    if let Some((class, observation)) = execution {
        findings.push(finding(
            key,
            class,
            "execution_status",
            &status,
            observation,
        ));
    }
    findings.sort_by(|left, right| left.fingerprint.cmp(&right.fingerprint));
    findings.dedup_by(|left, right| left.fingerprint == right.fingerprint);
    (findings, unknowns.into_iter().collect())
}

/// The runs' recorded failures (`runs[].failures[]` and each retried
/// attempt's `runs[].retry_attempts[].failures[]`) as
/// `sentinel::reports::ingest` observations. Only what the E2E recorded is
/// forwarded: a score loss without a failure record is never turned into an
/// error. The E2E scenario contract states no expected errors, so
/// `expected_errors` is `null` (Sentinel: contract unknown).
pub fn sentinel_observations(
    evidence: &ExecutionEvidenceV1,
    side: &str,
    revision: &str,
    at_ms: i64,
) -> Vec<Value> {
    let mut observations = Vec::new();
    for (index, report) in reports(&evidence.bundle).enumerate() {
        for scenario in report["report"]["scenarios"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let scenario_id = text_at(scenario, "/scenario_id").unwrap_or_default();
            for run in scenario["runs"].as_array().into_iter().flatten() {
                let run_id = text_at(run, "/run_id").unwrap_or_default();
                let path =
                    format!("detail/reports/{index}/report/scenarios/{scenario_id}/runs/{run_id}");
                // The final attempt and every attempt the E2E retried before it.
                let attempts = std::iter::once((run, path.clone())).chain(
                    retry_attempts(run)
                        .iter()
                        .enumerate()
                        .map(|(retry, attempt)| {
                            (attempt, format!("{path}/retry_attempts/{retry}"))
                        }),
                );
                for (attempt, path) in attempts {
                    for (occurrence, failure) in attempt["failures"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .enumerate()
                    {
                        let message = text_at(failure, "/message").unwrap_or_default();
                        let code = text_at(failure, "/code");
                        if message.is_empty() && code.is_none() {
                            continue;
                        }
                        observations.push(serde_json::json!({
                            "execution_id": evidence.execution_id,
                            "run_id": run_id,
                            "attempt": attempt["attempt_number"].as_u64().unwrap_or(1),
                            "scenario_id": scenario_id,
                            "phase": text_at(failure, "/phase").unwrap_or_else(|| "unknown".into()),
                            "side": side,
                            "revision": revision,
                            "artifact": format!("{path}/failures"),
                            "occurrence_index": occurrence,
                            "at_ms": at_ms,
                            "service_name": text_at(failure, "/domain").unwrap_or_else(|| "e2e".into()),
                            "exception_type": code.or_else(|| text_at(failure, "/status")),
                            "message": message,
                            "session_id": text_at(attempt, "/session_id"),
                            "expected_errors": Value::Null,
                            "excerpts": [],
                        }));
                    }
                }
            }
        }
    }
    observations
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn bundle(binaries: &[Option<&str>], manifest: Option<&str>) -> Value {
        let reports: Vec<Value> = binaries
            .iter()
            .map(|binary| {
                json!({"scenario_id": "s", "report": {"system_under_test": {
                    "harness_version": "h", "harness_binary_sha256": binary}, "scenarios": []}})
            })
            .collect();
        let mut bundle =
            json!({"manifest": {"executions": [{"id": "x"}]}, "detail": {"reports": reports}});
        if let Some(manifest) = manifest {
            bundle["manifest"]["system_under_test"] = json!({"harness_binary_sha256": manifest});
        }
        bundle
    }

    #[test]
    fn the_transcript_handed_on_is_a_run_that_lost_the_criterion() {
        let run = |id: &str, award: u8, reply: &str| {
            json!({"run_id": id, "criteria": [{"id": "two_turns", "possible": 10, "awarded": award}],
                "transcript": {"messages": [
                    {"message": {"role": "user", "content": [{"type": "text", "text": "Store  it\nonce."}]}},
                    {"custom": {"custom_type": "runtime_context"}},
                    {"message": {"role": "assistant", "content": [{"type": "thinking"},
                        {"type": "function_call", "function_id": "agent_trigger",
                         "arguments": {"function": "directory::search", "payload": {"q": "state"}}}]}},
                    {"message": {"role": "assistant", "content": [{"type": "text", "text": reply}]}}]}})
        };
        let bundle = json!({"detail": {"reports": [{"report": {"scenarios": [
            {"scenario_id": "s", "runs": [run("kept", 10, "fine"), run("lost", 0, "MIN-DONE")]}]}}]}});
        let (run_id, text) = transcript_excerpt(&bundle, "s", Some("two_turns"), 1000).unwrap();
        assert_eq!(run_id, "lost");
        assert_eq!(
            text,
            "user: Store it once.\n(notice: runtime_context)\nassistant: (thinking) | calls directory::search {\"function\":\"directory::search\",\"payload\":{\"q\":\"state\"}}\nassistant: MIN-DONE\n"
        );
        // A limit cuts at a line, and a scenario without transcripts has none.
        let (_, cut) = transcript_excerpt(&bundle, "s", Some("two_turns"), 30).unwrap();
        assert_eq!(cut, "user: Store it once.\n…");
        assert!(transcript_excerpt(&bundle, "other", None, 1000).is_none());
    }

    #[test]
    fn workers_built_from_each_sides_commit_leave_the_fixed_stack_alike() {
        let stack = |ide: &str, state: &str| {
            json!({"mode": "registry", "stack_versions": {"harness": "@0068cc1",
                "ide": ide, "state": state, "harness-e2e": "@5ecf8be"}})
        };
        assert_eq!(
            fixed_stack_sha256(&stack("@0068cc1", "0.22.22")),
            fixed_stack_sha256(&stack("@3a05d1b", "0.22.22"))
        );
        assert_ne!(
            fixed_stack_sha256(&stack("@0068cc1", "0.22.22")),
            fixed_stack_sha256(&stack("@0068cc1", "0.22.23"))
        );
    }

    #[test]
    fn a_github_campaign_attests_the_commit_it_built_by_its_resolved_stack() {
        let digest = format!("sha256:{}", "a".repeat(64));
        let system = json!({"stack": {"stack_versions": {"harness": "@0068cc1"}},
            "harness_binary_sha256": digest});
        assert_eq!(github_built_revision(&system).as_deref(), Some("0068cc1"));
        // A released version, or no built binary, attests no commit.
        assert_eq!(
            github_built_revision(&json!({"stack": {"stack_versions": {"harness": "1.8.62"}},
                "harness_binary_sha256": digest})),
            None
        );
        assert_eq!(
            github_built_revision(&json!({"stack": {"stack_versions": {"harness": "@0068cc1"}}})),
            None
        );
    }

    #[test]
    fn function_call_errors_are_counted_on_the_transcript_not_the_harness_total() {
        let result =
            |is_error: bool| json!({"message": {"role": "function_result", "is_error": is_error}});
        let bundle = json!({"detail": {"reports": [{"report": {"scenarios": [{"scenario_id": "s", "runs": [
            {"run_id": "a", "efficiency": {"function_call_errors": 0},
             "transcript": {"messages": [result(true), result(false), result(true)]}},
            {"run_id": "b", "efficiency": {"function_call_errors": 4}}]}]}}]}});
        let errors: Vec<_> = runs_of(&bundle, "s", &[])
            .iter()
            .map(|run| run.function_call_errors)
            .collect();
        assert_eq!(errors, [Some(2.0), None]);
    }

    #[test]
    fn the_observed_harness_binary_is_known_only_when_every_report_agrees() {
        let binary = |bundle: Value| identity(&bundle, "x").harness_binary_sha256;
        assert_eq!(
            binary(bundle(&[Some("a"), Some("a")], Some("a"))).as_deref(),
            Some("a")
        );
        assert_eq!(
            binary(bundle(&[Some("a"), Some("a")], None)).as_deref(),
            Some("a")
        );
        assert_eq!(
            binary(bundle(&[Some("a"), Some("b")], None)),
            None,
            "ambiguous"
        );
        assert_eq!(
            binary(bundle(&[Some("a"), None], None)),
            None,
            "missing in one report"
        );
        assert_eq!(
            binary(bundle(&[Some("a")], Some("b"))),
            None,
            "the manifest disagrees"
        );
        assert_eq!(binary(bundle(&[], None)), None);
    }
}
