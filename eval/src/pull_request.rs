//! The loop's one outward effect: the pull request that puts an accepted
//! candidate up for review. It goes from the candidate's branch to `main`,
//! ready for review, with the evidence in its body. Nothing is merged,
//! released or activated: a person reviews and merges.
//!
//! The credential comes from the fixed variable [`GITHUB_TOKEN_ENV`], read at
//! the moment of each request, and the API origin from the worker's
//! bootstrap ([`crate::runtime::Deps`]), never from a request. Reasons never
//! carry remote text.

use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Method;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::contract::*;
use crate::error::EvalError;
use crate::runtime::{end, transition, Deps};

pub const GITHUB_API_URL: &str = "https://api.github.com";
pub const REPOSITORY: &str = "iii-hq/workers";
pub const BASE_BRANCH: &str = "main";
/// The controller's GitHub token.
pub const GITHUB_TOKEN_ENV: &str = "EVAL_GITHUB_TOKEN";

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const REASON_MAX: usize = 300;

/// The pull request of an accepted candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PullRequestV1 {
    pub number: u64,
    pub url: String,
    pub head_ref: String,
    pub head_sha: String,
    /// Opened by this cycle rather than found.
    pub created: bool,
}

/// Why no pull request came back. `retry`: the answer was lost or GitHub
/// failed (5xx); asking again finds what a lost call opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub retry: bool,
    pub reason: String,
}

impl Refusal {
    fn stop(reason: impl Into<String>) -> Self {
        Self {
            retry: false,
            reason: truncate(&reason.into()),
        }
    }
    fn retry(reason: impl Into<String>) -> Self {
        Self {
            retry: true,
            reason: truncate(&reason.into()),
        }
    }
}

/// Reuses the open or merged pull request whose head is exactly
/// `candidate_sha`, or opens one ready for review. The branch must already
/// point at the candidate.
pub async fn ensure(
    api_url: &str,
    branch: &str,
    candidate_sha: &str,
    title: &str,
    body: &str,
) -> Result<PullRequestV1, Refusal> {
    if !is_git_sha(candidate_sha) {
        return Err(Refusal::stop("the candidate is not a full git sha"));
    }
    if branch.is_empty()
        || !branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
    {
        return Err(Refusal::stop(
            "the candidate branch is not a branch-safe name",
        ));
    }
    let github = GitHub::new(api_url)?;
    let (status, head) = github
        .call(Method::GET, &format!("/git/ref/heads/{branch}"), None)
        .await?;
    if status == 404 {
        return Err(Refusal::stop(format!("branch {branch} does not exist")));
    }
    expect_status(status, 200, "GitHub branch read")?;
    let branch_sha = text(&head, "/object/sha").unwrap_or_default();
    if branch_sha != candidate_sha {
        return Err(Refusal::stop(format!(
            "branch {branch} is at {}, not the accepted candidate {candidate_sha}",
            shown(branch_sha)
        )));
    }
    let owner = REPOSITORY.split('/').next().unwrap_or_default();
    let (status, pulls) = github
        .call(
            Method::GET,
            &format!("/pulls?head={owner}:{branch}&state=all&per_page=100"),
            None,
        )
        .await?;
    expect_status(status, 200, "GitHub pull request search")?;
    let pulls = pulls.as_array().cloned().unwrap_or_default();
    let exact = |pull: &&Value| text(pull, "/head/sha") == Some(candidate_sha);
    let found = pulls
        .iter()
        .filter(exact)
        .find(|pull| text(pull, "/state") == Some("open") || !pull["merged_at"].is_null());
    if let Some(pull) = found {
        return evidence(pull, candidate_sha, false);
    }
    let (status, pull) = github
        .call(
            Method::POST,
            "/pulls",
            Some(&json!({
                "title": title,
                "head": branch,
                "base": BASE_BRANCH,
                "body": body,
                "draft": false,
            })),
        )
        .await?;
    expect_status(status, 201, "GitHub pull request creation")?;
    evidence(&pull, candidate_sha, true)
}

/// The step after an accepted source decision: open the pull request, or
/// end `decided` saying why none was opened. A lost answer waits for the
/// next sweep; the cycle's limits bound the wait.
pub async fn stage(
    deps: &Deps,
    record: &CycleRecordV1,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let accepted = record
        .decision
        .as_ref()
        .is_some_and(|decision| decision.outcome == DecisionOutcomeV1::Accepted);
    let candidate = record.candidate.as_ref();
    let commit = candidate
        .and_then(|state| state.candidate.as_ref())
        .and_then(|candidate| candidate.commit.as_ref());
    let publish = candidate.and_then(|state| state.publish.as_ref());
    let (Some(commit), Some(publish)) = (commit.filter(|_| accepted), publish) else {
        let branch = candidate.map_or("", |state| state.identity.branch.as_str());
        return end(
            deps,
            record,
            OutcomeCodeV1::Decided,
            &format!(
                "accepted, no pull request: the candidate branch {branch} was not published (config.candidate.publish_remote)"
            ),
        )
        .await;
    };
    let ledger = crate::state::get_ledger(&deps.iii, &record.cycle_id).await?;
    let body = body(record, commit, ledger.as_ref());
    let title = title(record);
    match ensure(
        &deps.github_api_url(),
        &publish.branch,
        &commit.candidate_sha,
        &title,
        &body,
    )
    .await
    {
        Ok(pull) => {
            let detail = format!(
                "pull request #{} ready for review: {}",
                pull.number, pull.url
            );
            crate::runtime::transition_with(deps, record, |next| {
                next.pull_request = Some(pull.clone());
                next.outcome = Some(CycleOutcomeV1 {
                    code: OutcomeCodeV1::Proposed,
                    detail: detail.clone(),
                });
                Ok((CycleStatusV1::Completed, CycleStageV1::Done, detail.clone()))
            })
            .await
        }
        Err(refusal) if refusal.retry => {
            transition(
                deps,
                &record.cycle_id,
                record.step,
                CycleStatusV1::Waiting,
                CycleStageV1::PullRequest,
                &format!(
                    "pull request not opened yet: {}; asked again on the next sweep",
                    refusal.reason
                ),
                |_| Ok(()),
            )
            .await
        }
        Err(refusal) => {
            end(
                deps,
                record,
                OutcomeCodeV1::Decided,
                &format!(
                    "accepted, no pull request: {}; the change is on branch {}",
                    refusal.reason, publish.branch
                ),
            )
            .await
        }
    }
}

fn title(record: &CycleRecordV1) -> String {
    let change = record
        .proposal
        .as_ref()
        .and_then(|state| state.proposal.as_ref())
        .and_then(|proposal| proposal.change.as_deref())
        .unwrap_or("loop candidate");
    let line = change.lines().next().unwrap_or(change).trim();
    let mut title: String = line.chars().take(72).collect();
    if title.len() < line.len() {
        title.push('…');
    }
    format!("eval: {title}")
}

/// The evidence a reviewer needs, from the record alone: what was asked,
/// what changed, what each criterion measured on each side, what the
/// transcripts showed, where the change runs, and what it cost.
pub fn body(
    record: &CycleRecordV1,
    commit: &crate::candidate::CandidateCommitV1,
    ledger: Option<&CycleLedgerV1>,
) -> String {
    let mut out = String::new();
    let proposal = record
        .proposal
        .as_ref()
        .and_then(|state| state.proposal.as_ref());
    if let Some(plan) = &record.plan {
        out.push_str(&format!("Objective: {}\n\n", plan.objective));
    }
    if let Some(proposal) = proposal {
        if let Some(hypothesis) = &proposal.hypothesis {
            out.push_str(&format!("Hypothesis: {hypothesis}\n\n"));
        }
        if let Some(change) = &proposal.change {
            out.push_str(&format!("Change: {change}\n\n"));
        }
    }
    if let Some(decision) = &record.decision {
        out.push_str("### Measured\n\n");
        out.push_str(&format!("{}\n\n", decision.reason));
        out.push_str(
            "| | Criterion | Scenario | Baseline | Candidate | Runs | Result |\n|---|---|---|---|---|---|---|\n",
        );
        // The plan's criteria in the order the decision judged them: the
        // goal first, then each kept condition.
        let criteria = record.plan.iter().flat_map(|plan| {
            std::iter::once(("goal", &plan.criterion))
                .chain(plan.preserved.iter().map(|criterion| ("kept", criterion)))
        });
        let judged = std::iter::once(&decision.primary).chain(&decision.preserved);
        for ((role, criterion), evidence) in criteria.zip(judged) {
            let mean = |side: &EvidenceSideV1| {
                side.mean
                    .map_or("unmeasured".to_string(), |mean| format!("{mean:.3}"))
            };
            let name = criterion
                .criterion_id
                .clone()
                .unwrap_or_else(|| format!("{:?}", criterion.metric));
            out.push_str(&format!(
                "| {role} | {name} | {} | {} | {} | {} / {} | {:?} |\n",
                evidence.scenario_id,
                mean(&evidence.baseline),
                mean(&evidence.candidate),
                evidence.baseline.n,
                evidence.candidate.n,
                evidence.computed_outcome,
            ));
        }
        out.push_str(&format!(
            "\nE2E executions: baseline `{}`, candidate `{}`. Policy `{}`.\n\n",
            decision.baseline_execution_id,
            decision.candidate_execution_id,
            decision.policy_revision
        ));
    }
    let findings: Vec<String> = record
        .analyses
        .iter()
        .filter(|analysis| {
            matches!(
                analysis.role,
                ExecutionRoleV1::Baseline | ExecutionRoleV1::Candidate | ExecutionRoleV1::Control
            )
        })
        .map(|analysis| {
            let counts = crate::runtime::class_counts(analysis);
            format!(
                "- {:?} `{}`: {}",
                analysis.role,
                analysis.execution_id,
                if counts.is_empty() {
                    "no finding".to_string()
                } else {
                    counts
                }
            )
        })
        .collect();
    if !findings.is_empty() {
        out.push_str("### Transcript findings\n\n");
        out.push_str(&findings.join("\n"));
        out.push_str("\n\n");
    }
    out.push_str("### Where it runs\n\n");
    let built = record
        .config
        .as_ref()
        .and_then(|stored| stored.config.validation.as_ref())
        .map(ValidationConfigV1::built_from_commit)
        .unwrap_or_default();
    let measured = commit.measured_in(&built);
    let unmeasured: Vec<&String> = commit
        .units
        .iter()
        .chain(&commit.dependents)
        .filter(|unit| !measured.contains(unit) && !crate::candidate::is_shared(unit))
        .collect();
    out.push_str(&format!(
        "Touches {}. Measured in {} (built from `{}`).",
        list(&commit.units),
        list(&measured),
        commit.candidate_sha
    ));
    if !unmeasured.is_empty() {
        out.push_str(&format!(
            " Not measured: {}.",
            unmeasured
                .iter()
                .map(|unit| format!("`{unit}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let manifests: Vec<&str> = commit
        .paths
        .iter()
        .map(|path| path.path.as_str())
        .filter(|path| path.ends_with("iii.worker.yaml"))
        .collect();
    if !manifests.is_empty() {
        out.push_str(&format!(
            " Changes {}: the validation ran each worker's dependencies at their latest release, so a dependency change there was not measured.",
            manifests
                .iter()
                .map(|path| format!("`{path}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.push_str("\n\n");
    if let Some(ledger) = ledger {
        let usage = crate::cost::usage(ledger, crate::ids::now_ms());
        out.push_str(&format!(
            "Cycle `{}` spent ${:.2} over {} calls in {} min.\n",
            record.cycle_id,
            usage.spend_usd,
            usage.calls,
            usage.elapsed_ms / 60_000
        ));
    }
    out
}

fn list(units: &[String]) -> String {
    if units.is_empty() {
        return "nothing".into();
    }
    units
        .iter()
        .map(|unit| format!("`{unit}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

struct GitHub {
    http: reqwest::Client,
    api_url: String,
}

impl GitHub {
    fn new(api_url: &str) -> Result<Self, Refusal> {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent("iii-eval")
            .build()
            .map_err(|_| Refusal::stop("the http client could not be built"))?;
        Ok(Self {
            http,
            api_url: api_url.trim_end_matches('/').to_string(),
        })
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Value), Refusal> {
        let token = std::env::var(GITHUB_TOKEN_ENV)
            .ok()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                Refusal::stop(format!("credential variable {GITHUB_TOKEN_ENV} is not set"))
            })?;
        let mut request = self
            .http
            .request(method, format!("{}/repos/{REPOSITORY}{path}", self.api_url))
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(body) = body {
            request = request
                .header(CONTENT_TYPE, "application/json")
                .body(body.to_string());
        }
        // Only the kind: a transport error's text can carry a URL or a
        // peer's words. Asking again is safe: the search finds what a lost
        // call opened.
        let response = request
            .send()
            .await
            .map_err(|_| Refusal::retry("GitHub did not answer"))?;
        let status = response.status().as_u16();
        let bytes = response
            .bytes()
            .await
            .map_err(|_| Refusal::retry("the GitHub answer was lost"))?;
        if status >= 500 {
            return Err(Refusal::retry(format!("GitHub answered {status}")));
        }
        if status == 401 {
            return Err(Refusal::stop("the GitHub token is missing or expired"));
        }
        Ok((
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        ))
    }
}

/// A status other than the expected one. Only this module's own words and
/// the status go into the reason: a remote message can carry anything.
fn expect_status(status: u16, expected: u16, what: &str) -> Result<(), Refusal> {
    if status == expected {
        return Ok(());
    }
    Err(Refusal::stop(format!("{what} answered {status}")))
}

fn evidence(pull: &Value, candidate_sha: &str, created: bool) -> Result<PullRequestV1, Refusal> {
    let head_sha = text(pull, "/head/sha").unwrap_or_default();
    if head_sha != candidate_sha {
        return Err(Refusal::stop(format!(
            "the pull request head is {}, not the accepted candidate",
            shown(head_sha)
        )));
    }
    Ok(PullRequestV1 {
        number: pull["number"]
            .as_u64()
            .ok_or_else(|| Refusal::retry("the pull request has no number"))?,
        url: text(pull, "/html_url").unwrap_or_default().to_string(),
        head_ref: text(pull, "/head/ref").unwrap_or_default().to_string(),
        head_sha: head_sha.to_string(),
        created,
    })
}

/// A remote sha fit for a reason: a full git sha passes, anything else is
/// withheld.
fn shown(value: &str) -> &str {
    if is_git_sha(value) {
        value
    } else {
        "<withheld>"
    }
}

fn text<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer)?.as_str()
}

fn is_git_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn truncate(value: &str) -> String {
    match value.char_indices().nth(REASON_MAX) {
        Some((end, _)) => format!("{}…", &value[..end]),
        None => value.to_string(),
    }
}
