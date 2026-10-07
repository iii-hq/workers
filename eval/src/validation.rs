//! Starting a validation from a suggestion: two pushed commits of the Harness
//! become two Docker stacks and two E2E executions of one scenario; the sweep
//! waits for both, attaches the pair and computes its evidence. Nothing here
//! runs unless a person asks for it: every execution spends model tokens.

use std::process::{Command, Stdio};

use serde_json::{json, Value};

use crate::contract::*;
use crate::error::EvalError;
use crate::review;
use crate::runtime::{call, fetch_link, save_link, suggestion_at, Deps, BUS_TIMEOUT_MS};
use crate::{ids, state};

const DEFAULT_RUNS: u32 = 5;
/// Attempts the E2E repeats a run that failed for infrastructure reasons.
const TECHNICAL_RETRIES: u32 = 1;
/// The repository the E2E builds the pinned Harness from.
const REPOSITORY: &str = "iii-hq/workers";
/// The stack the executions start from: the one the E2E builds on its
/// `harness` template.
const BASE_TEMPLATE: &str = "harness";
const BASELINE_BRANCH: &str = "origin/main";
/// Characters of a commit shown to a person.
const SHORT_SHA: usize = 12;
pub const START_TIMEOUT_MS: u64 = 30_000;
/// A run still `starting` this long after it was recorded lost its answer
/// (the worker restarted, or the E2E never answered, between asking it and
/// saving what it said).
const STARTING_STALE_MS: i64 = 5 * 60 * 1_000;
/// The sweep stops trying to attach a pair this long after both executions
/// ended: a failure that lasts that long (an E2E that is down, a bundle too
/// large to read) will not pass by itself, and the person can attach by hand
/// or start again.
const ATTACH_GIVE_UP_MS: i64 = 30 * 60 * 1_000;
/// The states of a finished execution in `executions-list` (the others are
/// `running`, `cancelling` and `importing`).
const FINISHED: [&str; 4] = ["completed", "interrupted", "cancelled", "failed"];

fn invalid(message: impl Into<String>) -> EvalError {
    EvalError::InvalidRequest(message.into())
}

/// `eval::start-validation`. Errors carry stable prefixes:
/// `code_repository_required:`, `git_unavailable:`, `git_ref_invalid:`,
/// `commit_not_pushed:`, `e2e_unavailable:`, `e2e_busy:` and
/// `e2e_start_unconfirmed:` (the E2E did not answer, so an execution may have
/// started: the run stays in progress until the sweep settles it).
///
/// With `dry_run` it stops after the checks that need no E2E: the refs
/// resolve, both commits are pushed and differ, and the scenario and runs are
/// valid. It answers with the resolved commits and changes nothing.
pub async fn start_validation(
    deps: &Deps,
    request: StartValidationRequestV1,
) -> Result<StartValidationResponseV1, EvalError> {
    let runs = request.runs.unwrap_or(DEFAULT_RUNS);
    if !(1..=review::MAX_RUNS).contains(&runs) {
        return Err(invalid(format!(
            "runs must be between 1 and {}",
            review::MAX_RUNS
        )));
    }
    let scenario_id = review::required("scenario_id", Some(&request.scenario_id), 200)?;
    if let Some(criterion) = request.criterion.as_ref().filter(|c| c.min_runs > runs) {
        return Err(invalid(format!(
            "the criterion needs {} completed runs on each side but only {runs} are requested",
            criterion.min_runs
        )));
    }
    let evaluation_id = request.evaluation_id.as_str();
    let index = request.suggestion_index;
    let repository = state::get_config(&deps.iii)
        .await?
        .and_then(|config| config.code_repository)
        .ok_or_else(|| {
            invalid(
                "code_repository_required: configure the codebase directory (eval::configure \
                 code_repository) so the commits can be resolved",
            )
        })?;

    // What changes nothing comes first: a request that cannot start, a ref
    // that is not pushed or an E2E that cannot answer never costs anything.
    let (_, assets) = review::terminal_analysis(deps, evaluation_id, "start a validation").await?;
    suggestion_at(&assets, index)?;
    let (candidate_ref, baseline_ref) = (request.candidate_ref, request.baseline_ref);
    let resolution = {
        let repository = repository.clone();
        tokio::task::spawn_blocking(move || {
            resolve_commits(&repository, &candidate_ref, baseline_ref.as_deref())
        })
        .await
        .map_err(|error| EvalError::State(format!("git did not finish: {error}")))??
    };
    if request.dry_run {
        return Ok(StartValidationResponseV1::Resolved(resolution));
    }
    let (baseline_commit, candidate_commit) =
        (resolution.baseline.commit, resolution.candidate.commit);
    let model = review::required("model", Some(&request.model), 200)?;
    let provider = review::required("provider", Some(&request.provider), 200)?;
    let criterion = request
        .criterion
        .as_ref()
        .ok_or_else(|| invalid("criterion is required"))?;
    let by = review::author(
        request.by.as_deref(),
        deps.host_user.as_deref(),
        request.caller_worker_id.as_deref(),
    )?;
    let base = base_stack(deps).await?;
    let stack = |side: &str, commit: &str| -> Result<Value, EvalError> {
        Ok(json!({
            "name": stack_name(evaluation_id, index, side, commit),
            "yaml": stack_yaml(&base, commit)?,
        }))
    };
    let stacks = [
        ("baseline", stack("baseline", &baseline_commit)?),
        ("candidate", stack("candidate", &candidate_commit)?),
    ];

    // The criterion is registered, and the run recorded, before any
    // execution exists: nothing a result shows can shape them.
    let now = ids::now_ms();
    {
        let _guard = deps.locks.guard(evaluation_id).await;
        let (_, assets) =
            review::terminal_analysis(deps, evaluation_id, "start a validation").await?;
        let mut row = review::row_for(deps, &assets, evaluation_id, index).await?;
        if row.run.as_ref().is_some_and(|run| {
            matches!(
                run.state,
                ValidationRunStateV1::Starting
                    | ValidationRunStateV1::Running
                    | ValidationRunStateV1::Finished
            )
        }) {
            return Err(EvalError::Conflict(
                "a validation run of this suggestion is still in progress".into(),
            ));
        }
        let results_exist = review::data_exists(&row, &assets);
        review::register_criterion(
            &mut row,
            criterion,
            Some(&scenario_id),
            results_exist,
            by,
            now,
        )?;
        // The run this replaces may have had executions whose results were
        // seen: when results first existed does not move forward.
        if let Some(earlier) = row
            .run
            .as_ref()
            .filter(|run| {
                run.baseline_execution_id.is_some() || run.candidate_execution_id.is_some()
            })
            .map(|run| run.started_at)
        {
            row.first_run_at = Some(row.first_run_at.map_or(earlier, |first| first.min(earlier)));
        }
        row.run = Some(ValidationRunV1 {
            baseline_execution_id: None,
            candidate_execution_id: None,
            baseline_commit,
            candidate_commit,
            scenario_id: scenario_id.clone(),
            runs,
            model: model.clone(),
            provider: provider.clone(),
            started_at: now,
            finished_at: None,
            state: ValidationRunStateV1::Starting,
            error: None,
        });
        state::put_review(&deps.iii, &row).await?;
    }

    let mut started: Vec<String> = Vec::new();
    for (side, stack) in stacks {
        let parameters = json!({
            "scenarios": [scenario_id],
            "runs": runs,
            "technical_retries": TECHNICAL_RETRIES,
            "model": model,
            "provider": provider,
            "where": "docker",
            "stack": stack,
        });
        // The label tells whose execution it is in the E2E's own list.
        let label = format!("eval {evaluation_id} S{} {side}", index + 1);
        match start_execution(deps, &label, parameters).await {
            Ok(execution_id) => started.push(execution_id),
            Err((message, unconfirmed)) => {
                let message = match started.first() {
                    Some(baseline) => format!(
                        "{message}; the baseline execution {baseline} was already started and \
                         keeps running"
                    ),
                    None => message,
                };
                let baseline = started.first().cloned();
                let recorded = message.clone();
                // An answer that never came may hide a started execution: the
                // run stays `starting` (the sweep settles it, and meanwhile no
                // second start can duplicate it). Only a refusal is final.
                settle(
                    deps,
                    evaluation_id,
                    index,
                    ValidationRunStateV1::Starting,
                    |run| {
                        run.baseline_execution_id = baseline;
                        if !unconfirmed {
                            run.state = ValidationRunStateV1::Failed;
                            run.error = Some(recorded);
                        }
                    },
                )
                .await?;
                return Err(if unconfirmed {
                    EvalError::Unanswered(message)
                } else {
                    EvalError::Dependency(message)
                });
            }
        }
    }
    let (baseline, candidate) = (started[0].clone(), started[1].clone());
    let (b, c) = (baseline.clone(), candidate.clone());
    let row = settle(
        deps,
        evaluation_id,
        index,
        ValidationRunStateV1::Starting,
        |run| {
            run.baseline_execution_id = Some(b);
            run.candidate_execution_id = Some(c);
            run.state = ValidationRunStateV1::Running;
        },
    )
    .await?;
    row.map(|row| StartValidationResponseV1::Started(Box::new(row)))
        .ok_or_else(|| {
            EvalError::Conflict(format!(
                "the validation run changed while it started; the executions {baseline} and \
             {candidate} were started"
            ))
        })
}

/// Changes the suggestion's run under the analysis lock, only while it is
/// still in `expected`; `None` when it moved on.
async fn settle(
    deps: &Deps,
    evaluation_id: &str,
    index: usize,
    expected: ValidationRunStateV1,
    change: impl FnOnce(&mut ValidationRunV1),
) -> Result<Option<SuggestionReviewV1>, EvalError> {
    let _guard = deps.locks.guard(evaluation_id).await;
    let Some(mut row) = state::get_review(&deps.iii, evaluation_id, index).await? else {
        return Ok(None);
    };
    match row.run.as_mut().filter(|run| run.state == expected) {
        Some(run) => change(run),
        None => return Ok(None),
    }
    state::put_review(&deps.iii, &row).await?;
    Ok(Some(row))
}

/// The execution id, or why it did not start and whether the E2E never
/// answered (an execution may exist) rather than refused.
async fn start_execution(
    deps: &Deps,
    label: &str,
    parameters: Value,
) -> Result<String, (String, bool)> {
    let reply: Value = call(
        deps,
        "e2e::dashboard::execution-start",
        json!({ "label": label, "parameters": parameters }),
        deps.start_timeout_ms,
    )
    .await
    .map_err(|error| {
        let unanswered = matches!(error, EvalError::Unanswered(_));
        let text = error.to_string();
        let lowered = text.to_lowercase();
        let busy = [
            "busy",
            "already running",
            "another execution",
            "in progress",
        ]
        .iter()
        .any(|word| lowered.contains(word));
        if unanswered {
            return (
                format!(
                    "e2e_start_unconfirmed: the E2E did not answer, so the execution may have \
                     started anyway: look for `{label}` in the E2E before starting again ({text})"
                ),
                true,
            );
        }
        (
            format!(
                "{}: the E2E did not start the execution: {text}",
                if busy { "e2e_busy" } else { "e2e_unavailable" }
            ),
            false,
        )
    })?;
    reply["execution_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            (
                "e2e_unavailable: e2e::dashboard::execution-start returned no execution_id".into(),
                false,
            )
        })
}

// ---------------------------------------------------------------------------
// Commits and stacks
// ---------------------------------------------------------------------------

/// Runs git read-only in the code directory; `None` when it exits non-zero.
fn git(repository: &str, args: &[&str]) -> Result<Option<String>, EvalError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            EvalError::Dependency(format!("git_unavailable: could not run git: {error}"))
        })?;
    Ok(output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string()))
}

fn is_commit(text: &str) -> bool {
    text.len() == 40 && text.chars().all(|char| char.is_ascii_hexdigit())
}

/// The full commit a branch, tag or commit name points to.
fn resolve_commit(repository: &str, name: &str, role: &str) -> Result<String, EvalError> {
    let name = name.trim();
    // Only plain names: a leading dash would be an option, and the rest of
    // git's revision syntax is not needed to name a pushed commit.
    let plain = !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('-')
        && name
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || "._/@~^-".contains(char));
    if !plain {
        return Err(invalid(format!(
            "git_ref_invalid: the {role} ref `{name}` is not a plain branch, tag or commit name"
        )));
    }
    git(
        repository,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{name}^{{commit}}"),
        ],
    )?
    .filter(|sha| is_commit(sha))
    .ok_or_else(|| {
        invalid(format!(
            "git_ref_invalid: the {role} ref `{name}` is not a commit of {repository}"
        ))
    })
}

/// Both refs as full commits, each on a remote branch: the E2E builds the
/// Harness from GitHub, so a local-only commit cannot run. Only the clone's
/// remote-tracking branches are read (nothing is fetched), so run `git fetch`
/// first when a push is recent.
// ponytail: any remote counts, not only iii-hq/workers; check the remote's URL
// if fork-only commits become a real mistake.
fn resolve_commits(
    repository: &str,
    candidate_ref: &str,
    baseline_ref: Option<&str>,
) -> Result<ValidationResolutionV1, EvalError> {
    let candidate = resolve_commit(repository, candidate_ref, "candidate")?;
    let baseline_ref = baseline_ref.map(str::trim).filter(|name| !name.is_empty());
    let baseline = match baseline_ref {
        Some(name) => resolve_commit(repository, name, "baseline")?,
        None => git(repository, &["merge-base", &candidate, BASELINE_BRANCH])?
            .filter(|sha| is_commit(sha))
            .ok_or_else(|| {
                invalid(format!(
                    "git_ref_invalid: {repository} has no merge base of the candidate and \
                     {BASELINE_BRANCH}; pass baseline_ref"
                ))
            })?,
    };
    if baseline == candidate {
        return Err(invalid(format!(
            "git_ref_invalid: baseline and candidate are the same commit {}; a candidate already \
             in {BASELINE_BRANCH} needs an explicit baseline_ref",
            &candidate[..7]
        )));
    }
    let pushed = |role: &str, sha: &str, named: &str| -> Result<ResolvedCommitV1, EvalError> {
        let branch = remote_branch(repository, sha, named)?.ok_or_else(|| {
            invalid(format!(
                "commit_not_pushed: the {role} commit {sha} is on no remote branch of \
                 {repository}; push it (and git fetch) so the E2E can build it"
            ))
        })?;
        Ok(ResolvedCommitV1 {
            commit: sha.into(),
            short: sha[..SHORT_SHA].into(),
            branch,
        })
    };
    let resolved_baseline = pushed(
        "baseline",
        &baseline,
        baseline_ref.unwrap_or(BASELINE_BRANCH),
    )?;
    let resolved_candidate = pushed("candidate", &candidate, candidate_ref.trim())?;
    let mut warnings = Vec::new();
    // The default baseline is a merge base: an ancestor by construction.
    if baseline_ref.is_some()
        && git(
            repository,
            &["merge-base", "--is-ancestor", &baseline, &candidate],
        )?
        .is_none()
    {
        warnings.push(format!(
            "the baseline {} is not an ancestor of the candidate {}: the comparison also carries \
             changes that are not the candidate's",
            resolved_baseline.short, resolved_candidate.short
        ));
    }
    Ok(ValidationResolutionV1 {
        baseline: resolved_baseline,
        candidate: resolved_candidate,
        warnings,
    })
}

/// A remote branch that holds `sha` (`None` when none does): the one called
/// `named`, with or without its remote, else the first listed.
fn remote_branch(repository: &str, sha: &str, named: &str) -> Result<Option<String>, EvalError> {
    let listing = git(repository, &["branch", "-r", "--contains", sha])?.unwrap_or_default();
    let branches: Vec<&str> = listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.contains("->"))
        .collect();
    Ok(branches
        .iter()
        .find(|branch| {
            **branch == named
                || branch
                    .split_once('/')
                    .is_some_and(|(_, name)| name == named)
        })
        .or(branches.first())
        .map(|branch| (*branch).to_string()))
}

/// The YAML of the E2E's stack built on the `harness` template.
async fn base_stack(deps: &Deps) -> Result<String, EvalError> {
    let list: Value = call(
        deps,
        "e2e::dashboard::stacks-list",
        json!({}),
        BUS_TIMEOUT_MS,
    )
    .await
    .map_err(|error| {
        EvalError::Dependency(format!(
            "e2e_unavailable: the E2E service could not list its stacks: {error}"
        ))
    })?;
    list["stacks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|stack| stack["template"] == BASE_TEMPLATE)
        .and_then(|stack| stack["yaml"].as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            EvalError::Dependency(format!(
                "e2e_unavailable: no E2E stack is built on the `{BASE_TEMPLATE}` template"
            ))
        })
}

/// `yaml` with only its `harness` container replaced by the Harness built
/// from `commit` of the workers repository.
fn stack_yaml(yaml: &str, commit: &str) -> Result<String, EvalError> {
    let broken = |why: String| {
        EvalError::Dependency(format!(
            "e2e_unavailable: the E2E's `{BASE_TEMPLATE}` stack cannot be pinned: {why}"
        ))
    };
    let mut stack: serde_yaml::Value =
        serde_yaml::from_str(yaml).map_err(|error| broken(error.to_string()))?;
    let containers = stack
        .get_mut("containers")
        .and_then(serde_yaml::Value::as_mapping_mut)
        .filter(|containers| containers.contains_key("harness"))
        .ok_or_else(|| broken("it declares no `harness` container".into()))?;
    containers.insert(
        "harness".into(),
        serde_yaml::to_value(json!({
            "worker": "package://harness",
            "commit": commit,
            "repository": REPOSITORY,
        }))
        .map_err(|error| broken(error.to_string()))?,
    );
    serde_yaml::to_string(&stack).map_err(|error| broken(error.to_string()))
}

/// A name that tells the stack's purpose and is unique to its commit.
fn stack_name(evaluation_id: &str, index: usize, side: &str, commit: &str) -> String {
    let short: String = evaluation_id
        .trim_start_matches("eval_")
        .chars()
        .take(8)
        .collect();
    format!("eval-{short}-s{}-{side}-{}", index + 1, &commit[..7])
}

// ---------------------------------------------------------------------------
// The sweep
// ---------------------------------------------------------------------------

/// Moves every unfinished run: waits for both executions, then attaches the
/// pair. One run's trouble never stops the others, and a run is advanced by
/// one sweep at a time (a slow fetch is not started again by the next tick).
pub async fn advance_runs(deps: &Deps) {
    let rows = match state::list_reviews(&deps.iii).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%error, "sweep could not list the suggestion reviews");
            return;
        }
    };
    let active = |row: &SuggestionReviewV1| {
        row.run.as_ref().is_some_and(|run| {
            !matches!(
                run.state,
                ValidationRunStateV1::Attached | ValidationRunStateV1::Failed
            )
        })
    };
    for row in rows.into_iter().filter(active) {
        let key = format!("run:{}:{}", row.evaluation_id, row.suggestion_index);
        if !deps.inflight.insert(&key) {
            continue;
        }
        let outcome = advance_run(deps, &row).await;
        deps.inflight.remove(&key);
        if let Err(error) = outcome {
            tracing::warn!(
                evaluation_id = %row.evaluation_id,
                suggestion_index = row.suggestion_index,
                %error,
                "sweep could not advance the validation run"
            );
        }
    }
}

async fn advance_run(deps: &Deps, row: &SuggestionReviewV1) -> Result<(), EvalError> {
    let Some(run) = &row.run else {
        return Ok(());
    };
    let (evaluation_id, index) = (row.evaluation_id.as_str(), row.suggestion_index);
    let (Some(baseline), Some(candidate)) =
        (&run.baseline_execution_id, &run.candidate_execution_id)
    else {
        if run.state == ValidationRunStateV1::Starting
            && ids::now_ms() - run.started_at > STARTING_STALE_MS
        {
            settle(
                deps,
                evaluation_id,
                index,
                ValidationRunStateV1::Starting,
                |run| {
                    run.state = ValidationRunStateV1::Failed;
                    run.error = Some(
                    "e2e_unavailable: the start did not finish (the worker restarted, or the E2E \
                     never answered); look for this suggestion's executions in the E2E"
                        .into(),
                );
                },
            )
            .await?;
        }
        return Ok(());
    };

    // A run finished before `finished_at` was kept is timed from its start.
    let mut finished_at = run.finished_at.unwrap_or(run.started_at);
    if run.state == ValidationRunStateV1::Running {
        let list: Value = call(
            deps,
            "e2e::dashboard::executions-list",
            json!({ "ids": [baseline, candidate] }),
            BUS_TIMEOUT_MS,
        )
        .await?;
        let state_of = |id: &String| -> Option<String> {
            let entries = list["executions"].as_array()?;
            let entry = entries.iter().find(|entry| entry["id"] == id.as_str())?;
            Some(
                entry["state"]
                    .as_str()
                    .or_else(|| entry["plan_execution"]["state"].as_str())
                    .unwrap_or_default()
                    .to_string(),
            )
        };
        let states = [("baseline", baseline), ("candidate", candidate)].map(|(side, id)| {
            state_of(id).ok_or_else(|| {
                format!(
                    "e2e_execution_not_found: the {side} execution {id} is no longer listed by \
                     the E2E"
                )
            })
        });
        if let Some(Err(message)) = states.iter().find(|state| state.is_err()) {
            let message = message.clone();
            settle(
                deps,
                evaluation_id,
                index,
                ValidationRunStateV1::Running,
                |run| {
                    run.state = ValidationRunStateV1::Failed;
                    run.error = Some(message);
                },
            )
            .await?;
            return Ok(());
        }
        if !states
            .iter()
            .flatten()
            .all(|state| FINISHED.contains(&state.as_str()))
        {
            return Ok(());
        }
        let now = ids::now_ms();
        let finished = settle(
            deps,
            evaluation_id,
            index,
            ValidationRunStateV1::Running,
            |run| {
                run.state = ValidationRunStateV1::Finished;
                run.finished_at = Some(now);
            },
        )
        .await?;
        if finished.is_none() {
            return Ok(());
        }
        finished_at = now;
    }

    // Both ended, whatever the way: the pair is attached and its evidence
    // says how many runs completed.
    let attached = async {
        let (link, baseline, candidate) = fetch_link(deps, index, baseline, candidate).await?;
        save_link(deps, evaluation_id, link, (&baseline, &candidate), true).await
    }
    .await;
    match attached {
        Ok(_) => {}
        // This pair can never be attached.
        Err(
            error
            @ (EvalError::NotFound(_) | EvalError::InvalidRequest(_) | EvalError::Conflict(_)),
        ) => {
            let message = error.to_string();
            settle(
                deps,
                evaluation_id,
                index,
                ValidationRunStateV1::Finished,
                |run| {
                    run.state = ValidationRunStateV1::Failed;
                    run.error = Some(message);
                },
            )
            .await?;
        }
        // The E2E did not answer: the next sweep tries again, for a while.
        Err(error) => {
            let message = error.to_string();
            if ids::now_ms() - finished_at > ATTACH_GIVE_UP_MS {
                settle(
                    deps,
                    evaluation_id,
                    index,
                    ValidationRunStateV1::Finished,
                    |run| {
                        run.state = ValidationRunStateV1::Failed;
                        run.error = Some(format!(
                            "attach_gave_up: both executions ended but the pair could not be \
                             attached for {} minutes (last error: {message}); attach it by hand \
                             or start again",
                            ATTACH_GIVE_UP_MS / 60_000
                        ));
                    },
                )
                .await?;
            } else if run.error.as_deref() != Some(message.as_str()) {
                settle(
                    deps,
                    evaluation_id,
                    index,
                    ValidationRunStateV1::Finished,
                    |run| run.error = Some(message),
                )
                .await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_harness_container_is_replaced() {
        let base =
            "# header\niii: latest\ntemplate: harness\ncontainers:\n  harness:\n    worker: \
                    package://harness\n    version: latest\n  harness-e2e:\n    worker: \
                    package://harness-e2e\n    version: latest\nstartup_timeout: 5m\n";
        let pinned = stack_yaml(base, &"a".repeat(40)).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&pinned).unwrap();
        assert_eq!(parsed["iii"], "latest");
        assert_eq!(parsed["template"], "harness");
        assert_eq!(parsed["startup_timeout"], "5m");
        assert_eq!(parsed["containers"]["harness-e2e"]["version"], "latest");
        let harness = &parsed["containers"]["harness"];
        assert_eq!(harness["worker"], "package://harness");
        assert_eq!(harness["commit"], "a".repeat(40).as_str());
        assert_eq!(harness["repository"], "iii-hq/workers");
        assert!(harness.get("version").is_none(), "{pinned}");
        // The harness container stays where it was.
        assert!(pinned.find("harness:").unwrap() < pinned.find("harness-e2e:").unwrap());

        for broken in [
            "containers: {}",
            "iii: latest",
            "not: [valid",
            "containers: []",
        ] {
            let error = stack_yaml(broken, &"a".repeat(40)).unwrap_err();
            assert!(error.to_string().contains("e2e_unavailable:"), "{error}");
        }
    }

    #[test]
    fn stack_names_say_whose_they_are() {
        assert_eq!(
            stack_name("eval_0123456789abcdef", 1, "baseline", &"b".repeat(40)),
            "eval-01234567-s2-baseline-bbbbbbb"
        );
    }

    #[test]
    fn refs_that_could_be_options_never_reach_git() {
        for name in [
            "",
            "-h",
            "--upload-pack=x",
            "a b",
            "a;b",
            "a\nb",
            "$(x)",
            "a`b`",
        ] {
            let error = resolve_commit("/nonexistent", name, "candidate").unwrap_err();
            assert!(
                error.to_string().contains("git_ref_invalid:"),
                "{name:?}: {error}"
            );
        }
    }
}
