//! Promotion of an approved Harness candidate to the official release, and
//! its activation on an explicit local target.
//!
//! Every helper performs one external read or action against the official
//! surface it names and returns the evidence it saw; the caller journals a
//! [`PromotionPhase`] before and after each effect, so a crash or a lost
//! response is resumed from what was recorded instead of being repeated:
//!
//! 1. GitHub: reuse or open the pull request for the exact candidate head,
//!    then fast-forward `main` to the candidate with a non-forced ref update
//!    (the candidate is one commit on the approved base, so GitHub refuses the
//!    update the moment `main` moves), and prove the new `main`.
//! 2. Release Control (`rc_session` cookie): preview the Harness rc deploy
//!    from the merged SHA, confirm the frozen plan, read the operation, then
//!    read the exact Registry version and hash its archive bytes.
//! 3. Release Control: preview the finalization of that rc, confirm, read,
//!    and require the stable archive to be the validated rc archive.
//! 4. iii Compose, through a client connected to the configured local target
//!    Engine: pin the stable version and prove the running executable's
//!    digest; roll back by pinning the validated prior version the same way.
//!
//! R1 releases one worker from one place: Harness, `iii-hq/workers`, `main`,
//! exact fast-forward only.
//!
//! Authority is not configurable per cycle: credentials come from the fixed
//! variables [`GITHUB_TOKEN_ENV`] and [`RELEASE_CONTROL_SESSION_ENV`], read at
//! the moment of each request, and service origins from [`PromotionOrigins`],
//! which the worker's bootstrap builds. Reasons never carry remote text.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, COOKIE};
use reqwest::Method;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// The one worker this promotion releases.
pub const WORKER: &str = "harness";
pub const OFFICIAL_REGISTRY_URL: &str = "https://api.workers.iii.dev";
pub const GITHUB_API_URL: &str = "https://api.github.com";
pub const REPOSITORY: &str = "iii-hq/workers";
pub const BASE_BRANCH: &str = "main";
/// The controller's GitHub token.
pub const GITHUB_TOKEN_ENV: &str = "EVAL_GITHUB_TOKEN";
/// The controller's Release Control `rc_session` cookie value.
pub const RELEASE_CONTROL_SESSION_ENV: &str = "EVAL_RELEASE_CONTROL_SESSION";
/// The deployment's authenticated Release Control API base (https).
pub const RELEASE_CONTROL_URL_ENV: &str = "EVAL_RELEASE_CONTROL_URL";
/// Release Control's session cookie.
pub const SESSION_COOKIE: &str = "rc_session";
/// How long a Release Control preview stays confirmable.
pub const PLAN_TTL_MS: u64 = 15 * 60 * 1000;

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const ARCHIVE_TIMEOUT: Duration = Duration::from_secs(300);
/// A Harness release archive is tens of megabytes; past this it is not one.
const ARCHIVE_MAX_BYTES: usize = 512 * 1024 * 1024;
const COMPOSE_TIMEOUT_MS: u64 = 60_000;
const REASON_MAX: usize = 300;

// ── errors ────────────────────────────────────────────────────────────────────

/// Why a step did not produce its evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StopKind {
    /// Missing or expired credentials, an unreachable service, or a target
    /// this module cannot observe.
    Unavailable,
    /// The outcome cannot be proved: a lost response, mergeability not yet
    /// computed, an unknown executable digest.
    Inconclusive,
    /// A concrete divergence or failed precondition. The promotion stops.
    Rejected,
}

/// Carries a reason fit for a journal: never a credential, never a request
/// or response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionError {
    pub kind: StopKind,
    pub reason: String,
}

impl PromotionError {
    fn new(kind: StopKind, reason: impl Into<String>) -> Self {
        Self {
            kind,
            reason: truncate(&reason.into()),
        }
    }
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::new(StopKind::Unavailable, reason)
    }
    pub fn inconclusive(reason: impl Into<String>) -> Self {
        Self::new(StopKind::Inconclusive, reason)
    }
    pub fn rejected(reason: impl Into<String>) -> Self {
        Self::new(StopKind::Rejected, reason)
    }
}

impl fmt::Display for PromotionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.kind {
            StopKind::Unavailable => "unavailable",
            StopKind::Inconclusive => "inconclusive",
            StopKind::Rejected => "rejected",
        };
        write!(f, "promotion {kind}: {}", self.reason)
    }
}

impl std::error::Error for PromotionError {}

type Step<T> = Result<T, PromotionError>;

// ── configuration ───────────────────────────────────────────────────────────────

/// What a cycle may choose about a promotion: which archive target to
/// digest and which local Engine to activate. Nothing here names a service
/// or a credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PromotionConfig {
    /// The Registry `binaries` key whose archive is digested, e.g.
    /// `x86_64-unknown-linux-gnu`.
    pub artifact_target: String,
    pub target: ActivationTarget,
}

/// The service origins the promotion talks to. Built by the worker's
/// bootstrap, never deserialized from a request: a cycle cannot point the
/// controller's credentials at another host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionOrigins {
    pub release_control_url: String,
    pub github_api_url: String,
    pub registry_url: String,
}

impl PromotionOrigins {
    /// Production origins: Release Control from the deployment's
    /// [`RELEASE_CONTROL_URL_ENV`], GitHub and the Registry pinned.
    pub fn from_deployment_env() -> Step<Self> {
        let release_control_url = std::env::var(RELEASE_CONTROL_URL_ENV)
            .ok()
            .filter(|url| url.starts_with("https://"))
            .ok_or_else(|| {
                PromotionError::unavailable(format!(
                    "{RELEASE_CONTROL_URL_ENV} is not set to an https URL"
                ))
            })?;
        Ok(Self {
            release_control_url,
            github_api_url: GITHUB_API_URL.into(),
            registry_url: OFFICIAL_REGISTRY_URL.into(),
        })
    }
}

/// The explicit local Engine whose Compose daemon runs the Harness to
/// activate. Never defaulted: it must not be the operator's own Harness by
/// accident. The caller connects the [`IIIClient`] it hands to
/// [`Promotion::new`] to `engine_url`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivationTarget {
    /// `ws://127.0.0.1:<port>`.
    pub engine_url: String,
    pub namespace: String,
    /// The compose file on the daemon host.
    pub compose_file: String,
}

impl ActivationTarget {
    /// Whether this module can read the target's executable: a loopback
    /// Engine on a Linux host. Anything else stays explicitly unknown.
    pub fn is_local_linux(&self) -> bool {
        let Some(rest) = self.engine_url.strip_prefix("ws://") else {
            return false;
        };
        let authority = rest.split('/').next().unwrap_or_default();
        let host = if let Some(bracketed) = authority.strip_prefix('[') {
            bracketed.split(']').next().unwrap_or_default()
        } else {
            authority.split(':').next().unwrap_or_default()
        };
        cfg!(target_os = "linux") && matches!(host, "127.0.0.1" | "localhost" | "::1")
    }
}

// ── journal ───────────────────────────────────────────────────────────────────

/// Everything one promotion did, in order. Persisted by the caller after
/// each phase, before the next effect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PromotionJournal {
    pub promotion_id: String,
    pub phases: Vec<PromotionPhase>,
}

impl PromotionJournal {
    pub fn new(promotion_id: impl Into<String>) -> Self {
        Self {
            promotion_id: promotion_id.into(),
            phases: Vec::new(),
        }
    }
    pub fn push(&mut self, phase: PromotionPhase) {
        self.phases.push(phase);
    }
    pub fn last(&self) -> Option<&PromotionPhase> {
        self.phases.last()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum PromotionPhase {
    SourceApproved(SourceApproval),
    PullRequest(PullRequestEvidence),
    MergeRequested(MergeRequested),
    Merged(MergeEvidence),
    RcPlanned(ReleasePlan),
    RcConfirmed(ConfirmedPlan),
    RcPublished(ReleaseArtifact),
    ArtifactApproved(ArtifactApproval),
    FinalizePlanned(ReleasePlan),
    FinalizeConfirmed(ConfirmedPlan),
    StablePublished(ReleaseArtifact),
    RollbackDestination(ValidatedArtifact),
    ActivationRequested(ComposeRequest),
    ActivationConfirmed(RunningEvidence),
    RollbackRequested(ComposeRequest),
    RollbackConfirmed(RunningEvidence),
    Stopped(Stopped),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Stopped {
    pub step: String,
    pub kind: StopKind,
    pub reason: String,
}

impl Stopped {
    pub fn at(step: impl Into<String>, error: &PromotionError) -> Self {
        Self {
            step: step.into(),
            kind: error.kind,
            reason: error.reason.clone(),
        }
    }
}

// ── approvals ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalOutcome {
    Accepted,
    Rejected,
    Inconclusive,
}

/// The frozen source comparison decision: it authorizes merging exactly
/// this candidate onto exactly this base, and nothing past the merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceApproval {
    pub candidate_id: String,
    pub outcome: ApprovalOutcome,
    pub policy_revision: String,
    pub base_sha: String,
    pub candidate_sha: String,
    pub candidate_tree: String,
    #[serde(default)]
    pub evidence_urls: Vec<String>,
}

/// The frozen decision on the immutable rc build, from an E2E run against
/// it. `binary_sha256` is the running executable the run measured
/// (`system_under_test.harness_binary_sha256`); absent is unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArtifactApproval {
    pub outcome: ApprovalOutcome,
    pub policy_revision: String,
    pub version: String,
    pub source_sha: String,
    pub archive_sha256: String,
    #[serde(default)]
    pub binary_sha256: Option<String>,
}

/// An immutable version whose archive and running executable were both
/// validated. The two digests are separate facts and never compared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValidatedArtifact {
    pub version: String,
    pub archive_sha256: String,
    pub binary_sha256: String,
}

fn outcome_accepted(outcome: ApprovalOutcome, what: &str) -> Step<()> {
    match outcome {
        ApprovalOutcome::Accepted => Ok(()),
        ApprovalOutcome::Rejected => Err(PromotionError::rejected(format!("{what} was rejected"))),
        ApprovalOutcome::Inconclusive => Err(PromotionError::inconclusive(format!(
            "{what} was inconclusive"
        ))),
    }
}

pub fn check_source_approval(approval: &SourceApproval) -> Step<()> {
    outcome_accepted(approval.outcome, "the source comparison")?;
    if approval.policy_revision.trim().is_empty() {
        return Err(PromotionError::rejected(
            "source approval names no policy revision",
        ));
    }
    for (field, value) in [
        ("base_sha", &approval.base_sha),
        ("candidate_sha", &approval.candidate_sha),
        ("candidate_tree", &approval.candidate_tree),
    ] {
        if !is_git_sha(value) {
            return Err(PromotionError::rejected(format!(
                "source approval {field} is not a full git sha"
            )));
        }
    }
    if approval.candidate_id.is_empty()
        || !approval
            .candidate_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(PromotionError::rejected(
            "candidate id is not a branch-safe name",
        ));
    }
    Ok(())
}

/// Bind the rc artifact approval to the exact build that was read back.
pub fn check_artifact_approval(
    approval: &ArtifactApproval,
    rc: &ReleaseArtifact,
) -> Step<ValidatedArtifact> {
    outcome_accepted(approval.outcome, "the rc artifact E2E")?;
    if approval.policy_revision.trim().is_empty() {
        return Err(PromotionError::rejected(
            "artifact approval names no policy revision",
        ));
    }
    if approval.version != rc.version {
        return Err(PromotionError::rejected(format!(
            "artifact approval is for {}, the rc is {}",
            approval.version, rc.version
        )));
    }
    if approval.source_sha != rc.source_sha {
        return Err(PromotionError::rejected(
            "artifact approval names a different source sha than the rc build",
        ));
    }
    if approval.archive_sha256 != rc.archive_sha256 {
        return Err(PromotionError::rejected(
            "artifact approval names a different archive digest than the rc build",
        ));
    }
    let Some(binary) = approval.binary_sha256.as_deref() else {
        return Err(PromotionError::inconclusive(
            "the running binary digest of the rc E2E is unknown",
        ));
    };
    if !is_sha256(binary) {
        return Err(PromotionError::rejected(
            "running binary digest is not a sha256",
        ));
    }
    // The E2E must have run the executable this archive installs, not a
    // cached build that happens to carry the same version.
    if binary != rc.binary_sha256 {
        return Err(PromotionError::rejected(format!(
            "the rc E2E ran executable {binary}, the {} archive installs {}",
            rc.version, rc.binary_sha256
        )));
    }
    Ok(ValidatedArtifact {
        version: rc.version.clone(),
        archive_sha256: rc.archive_sha256.clone(),
        binary_sha256: rc.binary_sha256.clone(),
    })
}

/// The stable release must be the validated rc bytes, republished. The
/// running binary digest carries over because the bytes are the same.
pub fn check_republished(
    rc: &ValidatedArtifact,
    stable: &ReleaseArtifact,
) -> Step<ValidatedArtifact> {
    if stable_for_candidate(&rc.version).as_deref() != Some(stable.version.as_str()) {
        return Err(PromotionError::rejected(format!(
            "{} is not the stable version of {}",
            stable.version, rc.version
        )));
    }
    if stable.archive_sha256 != rc.archive_sha256 {
        return Err(PromotionError::rejected(format!(
            "stable {} archive differs from the validated rc {} archive",
            stable.version, rc.version
        )));
    }
    if stable.binary_sha256 != rc.binary_sha256 {
        return Err(PromotionError::rejected(format!(
            "stable {} installs a different executable than the validated rc {}",
            stable.version, rc.version
        )));
    }
    Ok(ValidatedArtifact {
        version: stable.version.clone(),
        archive_sha256: stable.archive_sha256.clone(),
        binary_sha256: rc.binary_sha256.clone(),
    })
}

/// `X.Y.Z-rc.N` → `X.Y.Z`; anything else is not a candidate.
pub fn stable_for_candidate(version: &str) -> Option<String> {
    let (core, rc) = version.split_once("-rc.")?;
    let numeric = |part: &str| {
        !part.is_empty()
            && part.chars().all(|c| c.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'))
    };
    let parts: Vec<&str> = core.split('.').collect();
    (parts.len() == 3 && parts.iter().all(|part| numeric(part)) && numeric(rc) && rc != "0")
        .then(|| core.to_string())
}

pub fn candidate_branch(candidate_id: &str) -> String {
    format!("feat/loop-candidate-{candidate_id}")
}

/// The Compose operation id for one activation or rollback attempt: the
/// same inputs always name the same operation, so a restart reconciles it.
pub fn compose_operation_id(
    promotion_id: &str,
    action: &str,
    version: &str,
    attempt: u32,
) -> String {
    format!("loop-promotion:{promotion_id}:{action}:{version}:{attempt}")
}

// ── evidence ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PullRequestEvidence {
    pub number: u64,
    pub url: String,
    pub head_ref: String,
    pub head_sha: String,
    /// Opened by this call rather than found.
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MergeRequested {
    pub pull_request: u64,
    pub candidate_sha: String,
    /// False when `main` already was the candidate and nothing was sent.
    /// Either way the outcome is proved only by [`Promotion::read_merge`].
    pub updated: bool,
}

/// `main` fast-forwarded to the candidate: the merged commit is the
/// candidate commit, one parent on the approved base, with the approved
/// tree. `main_sha` is `main` when it was read back (the candidate, or a
/// later commit that contains it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MergeEvidence {
    pub pull_request: u64,
    pub base_sha: String,
    pub candidate_sha: String,
    pub candidate_tree: String,
    pub merged_sha: String,
    pub merged_tree: String,
    pub main_sha: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseKind {
    Deploy,
    FinalizeRelease,
}

impl ReleaseKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Deploy => "deploy",
            Self::FinalizeRelease => "finalize_release",
        }
    }
}

/// A Release Control preview frozen as it was answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReleasePlan {
    pub kind: ReleaseKind,
    pub plan_id: String,
    pub plan_hash: String,
    pub expires_at: String,
    pub source_sha: String,
    /// The version the operation will publish: the rc for a deploy, the
    /// stable version for a finalization.
    pub target_version: String,
    /// The rc a finalization promotes.
    pub candidate_version: Option<String>,
    pub channel: String,
    pub prior_next: Option<String>,
    pub prior_latest: Option<String>,
    /// sha256 of the preview document as received.
    pub preview_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfirmedPlan {
    pub plan: ReleasePlan,
    pub operation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeploymentEvidence {
    pub operation_id: String,
    pub deployment_id: String,
    pub version: String,
    pub channel: String,
    pub from_version: Option<String>,
    pub source_sha: String,
    pub descriptor_sha256: Option<String>,
    /// The latest successful build execution that carried a manifest.
    pub build_run_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OperationState {
    Converging {
        status: String,
    },
    /// Converged in Release Control's view. Not artifact proof: read the
    /// Registry with [`Promotion::read_artifact`].
    Converged(DeploymentEvidence),
    Blocked,
    Discarded,
}

/// An exact Registry version whose archive bytes were downloaded and
/// hashed, matching the digest the Registry claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReleaseArtifact {
    pub version: String,
    pub tags: Vec<String>,
    pub target: String,
    pub archive_url: String,
    pub archive_sha256: String,
    /// sha256 of the archive's one regular `harness` entry: the executable
    /// this release installs. A separate fact from `archive_sha256`.
    pub binary_sha256: String,
    pub source_sha: String,
    pub operation_id: String,
    pub deployment_id: String,
    pub build_run_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TargetObservation {
    pub file: String,
    pub namespace: String,
    pub container: String,
    pub state: String,
    pub version: Option<String>,
    /// The pid Compose reports: the supervised process, often a shell
    /// wrapper rather than Harness itself.
    pub supervisor_pid: Option<u32>,
    /// The Harness process found at or under the supervisor, whose
    /// executable was hashed.
    pub pid: Option<u32>,
    pub binary_sha256: Option<String>,
    pub binary_unknown_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "is", rename_all = "snake_case")]
pub enum TargetIs {
    Prior,
    Candidate,
    /// Running something that is neither: an operator change to respect.
    Other {
        reason: String,
    },
    Unknown {
        reason: String,
    },
}

/// A Compose pin this module asked for. `accepted: false` means the target
/// already ran the destination and nothing was sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ComposeRequest {
    pub operation_id: String,
    pub version: String,
    pub accepted: bool,
}

/// The target proved to run the expected executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunningEvidence {
    pub operation_id: String,
    pub version: String,
    /// The Compose-supervised process the Harness was found under.
    pub supervisor_pid: u32,
    /// The Harness process whose executable was hashed.
    pub pid: u32,
    pub binary_sha256: String,
    pub file: String,
    pub namespace: String,
}

/// A Compose operation as the target daemon reports it, read without
/// sending anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComposeOperationState {
    /// The daemon does not know the id (never submitted, or forgotten after
    /// a daemon restart), or reports a status this module does not know.
    /// Nothing about the target follows from it.
    Unknown,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Confirmation {
    Confirmed(RunningEvidence),
    Pending { reason: String },
}

pub fn classify_target(
    observed: &TargetObservation,
    prior: &ValidatedArtifact,
    candidate: &ValidatedArtifact,
) -> TargetIs {
    let Some(binary) = observed.binary_sha256.as_deref() else {
        return TargetIs::Unknown {
            reason: observed
                .binary_unknown_reason
                .clone()
                .unwrap_or_else(|| "the running executable digest is unknown".into()),
        };
    };
    let version = observed.version.as_deref();
    if binary == candidate.binary_sha256 && version == Some(candidate.version.as_str()) {
        return TargetIs::Candidate;
    }
    if binary == prior.binary_sha256 && version == Some(prior.version.as_str()) {
        return TargetIs::Prior;
    }
    TargetIs::Other {
        reason: format!(
            "{} runs {} with executable {binary}, neither the validated prior nor the candidate",
            observed.container,
            shown(version.unwrap_or("unpinned"))
        ),
    }
}

// ── effects ───────────────────────────────────────────────────────────────────

const COMPOSE_ADD: &str = "compose::add";
const COMPOSE_OPERATION: &str = "compose::operation";
const COMPOSE_STATUS: &str = "compose::status";

enum ComposeFailure {
    UnknownOperation,
    Stop(PromotionError),
}

/// The promotion's effects, bound to one configuration.
pub struct Promotion {
    config: PromotionConfig,
    origins: PromotionOrigins,
    http: reqwest::Client,
    /// Connected to `config.target.engine_url`.
    target_engine: Arc<IIIClient>,
}

impl Promotion {
    pub fn new(
        config: PromotionConfig,
        origins: PromotionOrigins,
        target_engine: Arc<IIIClient>,
    ) -> Step<Self> {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent("iii-eval-promotion")
            .build()
            .map_err(|_| PromotionError::unavailable("the http client could not be built"))?;
        Ok(Self {
            config,
            origins,
            http,
            target_engine,
        })
    }

    pub fn config(&self) -> &PromotionConfig {
        &self.config
    }

    // ── GitHub ─────────────────────────────────────────────────────────────────

    /// Reuse the open or merged pull request whose head is exactly the
    /// candidate, or open one carrying the evidence links. The branch must
    /// already point at the candidate.
    pub async fn ensure_pull_request(
        &self,
        approval: &SourceApproval,
    ) -> Step<PullRequestEvidence> {
        check_source_approval(approval)?;
        let branch = candidate_branch(&approval.candidate_id);
        let (status, head) = self
            .github(Method::GET, &format!("/git/ref/heads/{branch}"), None)
            .await?;
        if status == 404 {
            return Err(PromotionError::rejected(format!(
                "branch {branch} does not exist"
            )));
        }
        expect_status(status, 200, "GitHub branch read")?;
        let branch_sha = text(&head, "/object/sha").unwrap_or_default();
        if branch_sha != approval.candidate_sha {
            return Err(PromotionError::rejected(format!(
                "branch {branch} is at {}, not the approved candidate {}",
                shown(branch_sha),
                approval.candidate_sha
            )));
        }

        let owner = REPOSITORY.split('/').next().unwrap_or_default();
        let (status, pulls) = self
            .github(
                Method::GET,
                &format!("/pulls?head={owner}:{branch}&state=all&per_page=100"),
                None,
            )
            .await?;
        expect_status(status, 200, "GitHub pull request search")?;
        let exact =
            |pull: &&Value| text(pull, "/head/sha") == Some(approval.candidate_sha.as_str());
        let pulls = pulls.as_array().cloned().unwrap_or_default();
        let found = pulls
            .iter()
            .filter(exact)
            .find(|pull| text(pull, "/state") == Some("open"))
            .or_else(|| {
                pulls
                    .iter()
                    .filter(exact)
                    .find(|pull| !pull["merged_at"].is_null())
            });
        if let Some(pull) = found {
            return pull_evidence(pull, &approval.candidate_sha, false);
        }

        let mut body = format!(
            "Loop candidate `{}` approved under policy `{}`.\n\nBase `{}`, candidate `{}`, tree `{}`.",
            approval.candidate_id,
            approval.policy_revision,
            approval.base_sha,
            approval.candidate_sha,
            approval.candidate_tree
        );
        for url in &approval.evidence_urls {
            body.push_str(&format!("\n- {url}"));
        }
        let (status, pull) = self
            .github(
                Method::POST,
                "/pulls",
                Some(&json!({
                    "title": format!("loop candidate {}", approval.candidate_id),
                    "head": branch,
                    "base": BASE_BRANCH,
                    "body": body,
                })),
            )
            .await?;
        expect_status(status, 201, "GitHub pull request creation")?;
        pull_evidence(&pull, &approval.candidate_sha, true)
    }

    /// Fast-forward `main` to the candidate with a non-forced ref update.
    ///
    /// The candidate must be exactly one commit whose only parent is the
    /// approved base and whose tree is the approved tree; then GitHub can
    /// accept `force: false` only while `main` is still that base, so a race
    /// that moves `main` between the check and the update is refused by the
    /// server rather than merged. There is no merge commit, no squash and no
    /// other path: a refusal stops. A lost reply is reconciled by
    /// [`Self::read_merge`], and calling this again once `main` is the
    /// candidate sends nothing.
    pub async fn fast_forward_main(
        &self,
        approval: &SourceApproval,
        pull_request: u64,
    ) -> Step<MergeRequested> {
        check_source_approval(approval)?;
        self.check_candidate_commit(approval).await?;
        let (status, pull) = self
            .github(Method::GET, &format!("/pulls/{pull_request}"), None)
            .await?;
        expect_status(status, 200, "GitHub pull request read")?;
        if text(&pull, "/head/sha") != Some(approval.candidate_sha.as_str()) {
            return Err(PromotionError::rejected(format!(
                "pull request #{pull_request} head is not the approved candidate"
            )));
        }
        if text(&pull, "/base/ref") != Some(BASE_BRANCH) {
            return Err(PromotionError::rejected(format!(
                "pull request #{pull_request} does not target {BASE_BRANCH}"
            )));
        }
        let done = |updated| MergeRequested {
            pull_request,
            candidate_sha: approval.candidate_sha.clone(),
            updated,
        };
        let tip = self.main_tip().await?;
        if tip == approval.candidate_sha {
            return Ok(done(false));
        }
        if tip != approval.base_sha {
            return Err(PromotionError::rejected(format!(
                "{BASE_BRANCH} advanced from the approved base {} to {}",
                approval.base_sha,
                shown(&tip)
            )));
        }
        if text(&pull, "/state") != Some("open") {
            return Err(PromotionError::rejected(format!(
                "pull request #{pull_request} is closed"
            )));
        }
        match text(&pull, "/mergeable_state") {
            Some("clean") => {}
            Some("dirty") => {
                return Err(PromotionError::rejected(
                    "the pull request conflicts with its base",
                ))
            }
            Some("blocked") => {
                return Err(PromotionError::rejected(
                    "required checks or reviews are missing",
                ))
            }
            Some("behind") => {
                return Err(PromotionError::rejected(
                    "the pull request is behind its base",
                ))
            }
            Some("unstable") => return Err(PromotionError::rejected("checks are not passing")),
            _ => {
                return Err(PromotionError::inconclusive(
                    "GitHub has not settled mergeability",
                ))
            }
        }
        let (status, updated) = self
            .github(
                Method::PATCH,
                &format!("/git/refs/heads/{BASE_BRANCH}"),
                Some(&json!({ "sha": approval.candidate_sha, "force": false })),
            )
            .await?;
        if status == 200 {
            if text(&updated, "/object/sha") != Some(approval.candidate_sha.as_str()) {
                return Err(PromotionError::inconclusive(
                    "GitHub answered the update without the candidate; read the merge back",
                ));
            }
            return Ok(done(true));
        }
        // Refused. Why is read from the ref itself, never from GitHub's text.
        let now = self.main_tip().await?;
        if now == approval.candidate_sha {
            return Ok(done(false));
        }
        Err(PromotionError::rejected(if now != approval.base_sha {
            format!(
                "GitHub refused the fast-forward ({status}): {BASE_BRANCH} advanced to {}",
                shown(&now)
            )
        } else {
            format!(
                "GitHub refused the fast-forward of {BASE_BRANCH} ({status}); repository rules \
                 forbid the update and no other merge is attempted"
            )
        }))
    }

    /// Prove the fast-forward: the candidate is still one commit on the
    /// approved base with the approved tree, and `main` is the candidate or a
    /// later commit that contains it. `main` still on the base is
    /// inconclusive; anything else stops. Nothing is built before this holds.
    pub async fn read_merge(
        &self,
        approval: &SourceApproval,
        pull_request: u64,
    ) -> Step<MergeEvidence> {
        self.check_candidate_commit(approval).await?;
        let tip = self.main_tip().await?;
        if tip == approval.base_sha {
            return Err(PromotionError::inconclusive(format!(
                "{BASE_BRANCH} is still at the approved base; the fast-forward has not landed"
            )));
        }
        if tip != approval.candidate_sha {
            let (status, comparison) = self
                .github(
                    Method::GET,
                    &format!("/compare/{}...{tip}", approval.candidate_sha),
                    None,
                )
                .await?;
            expect_status(status, 200, "GitHub comparison")?;
            if text(&comparison, "/status") != Some("ahead") {
                return Err(PromotionError::rejected(format!(
                    "{BASE_BRANCH} is at {}, which does not contain the candidate",
                    shown(&tip)
                )));
            }
        }
        Ok(MergeEvidence {
            pull_request,
            base_sha: approval.base_sha.clone(),
            candidate_sha: approval.candidate_sha.clone(),
            candidate_tree: approval.candidate_tree.clone(),
            merged_sha: approval.candidate_sha.clone(),
            merged_tree: approval.candidate_tree.clone(),
            main_sha: tip,
        })
    }

    /// The candidate commit must be exactly one commit on the approved base
    /// carrying the approved tree.
    async fn check_candidate_commit(&self, approval: &SourceApproval) -> Step<()> {
        let (status, commit) = self
            .github(
                Method::GET,
                &format!("/git/commits/{}", approval.candidate_sha),
                None,
            )
            .await?;
        if status == 404 {
            return Err(PromotionError::rejected(
                "the candidate commit is not on GitHub",
            ));
        }
        expect_status(status, 200, "GitHub candidate commit read")?;
        let parents: Vec<&str> = commit["parents"]
            .as_array()
            .map(|parents| parents.iter().filter_map(|p| text(p, "/sha")).collect())
            .unwrap_or_default();
        if parents != [approval.base_sha.as_str()] {
            return Err(PromotionError::rejected(
                "the candidate is not a single commit whose only parent is the approved base",
            ));
        }
        if text(&commit, "/tree/sha") != Some(approval.candidate_tree.as_str()) {
            return Err(PromotionError::rejected(
                "the candidate commit does not carry the approved tree",
            ));
        }
        Ok(())
    }

    async fn main_tip(&self) -> Step<String> {
        let (status, reference) = self
            .github(Method::GET, &format!("/git/ref/heads/{BASE_BRANCH}"), None)
            .await?;
        expect_status(status, 200, "GitHub main read")?;
        text(&reference, "/object/sha")
            .filter(|sha| is_git_sha(sha))
            .map(str::to_string)
            .ok_or_else(|| PromotionError::inconclusive("GitHub answered no sha for main"))
    }

    // ── Release Control ──────────────────────────────────────────────────────

    /// Preview the Harness rc deploy from the merged commit. The plan must
    /// be unblocked and name exactly Harness, that source and `next`.
    pub async fn preview_rc(&self, merge: &MergeEvidence) -> Step<ReleasePlan> {
        let (status, envelope) = self
            .release_control(
                Method::POST,
                "/operations/preview",
                Some(&json!({
                    "type": "deploy",
                    "sourceSha": merge.merged_sha,
                    "workers": [{
                        "name": WORKER,
                        "version": { "mode": "bump", "bump": "patch", "suffix": "rc" },
                        "channel": "next",
                    }],
                })),
            )
            .await?;
        expect_status(status, 200, "Release Control preview")?;
        let preview = &envelope["preview"];
        let entry = single_entry(preview)?;
        if text(preview, "/sourceSha") != Some(merge.merged_sha.as_str())
            || text(entry, "/sourceSha") != Some(merge.merged_sha.as_str())
        {
            return Err(PromotionError::rejected(
                "the rc preview builds a different source than the merged commit",
            ));
        }
        if text(entry, "/channel") != Some("next") {
            return Err(PromotionError::rejected(
                "the rc preview does not target next",
            ));
        }
        let target_version = text(entry, "/targetVersion")
            .unwrap_or_default()
            .to_string();
        if stable_for_candidate(&target_version).is_none() {
            return Err(PromotionError::rejected(format!(
                "the rc preview targets {}, not an X.Y.Z-rc.N candidate",
                shown(&target_version)
            )));
        }
        frozen_plan(
            &envelope,
            ReleaseKind::Deploy,
            merge.merged_sha.clone(),
            target_version,
            None,
            "next",
            text(entry, "/snapshots/next/version").map(str::to_string),
            text(entry, "/snapshots/latest/version").map(str::to_string),
        )
    }

    /// Preview finalizing the approved rc. Release Control must still see
    /// that rc on `next`, from the approved source, with `latest` where the
    /// rc plan froze it.
    pub async fn preview_finalize(
        &self,
        rc: &ReleaseArtifact,
        approved: &ValidatedArtifact,
        rc_plan: &ReleasePlan,
    ) -> Step<ReleasePlan> {
        if approved.version != rc.version
            || approved.archive_sha256 != rc.archive_sha256
            || rc_plan.target_version != rc.version
            || rc_plan.source_sha != rc.source_sha
        {
            return Err(PromotionError::rejected(
                "the approved artifact, the rc build and the rc plan do not name the same release",
            ));
        }
        let stable = stable_for_candidate(&rc.version).ok_or_else(|| {
            PromotionError::rejected(format!("{} is not an X.Y.Z-rc.N candidate", rc.version))
        })?;
        let (status, envelope) = self
            .release_control(
                Method::POST,
                "/operations/preview",
                Some(&json!({
                    "type": "finalize_release",
                    "workers": [{ "name": WORKER, "candidateVersion": rc.version }],
                })),
            )
            .await?;
        expect_status(status, 200, "Release Control finalize preview")?;
        let entry = single_entry(&envelope["preview"])?;
        let expected_next = text(entry, "/expectedNextVersion").map(str::to_string);
        let expected_latest = text(entry, "/expectedLatestVersion").map(str::to_string);
        if text(entry, "/candidateVersion") != Some(rc.version.as_str())
            || text(entry, "/stableVersion") != Some(stable.as_str())
        {
            return Err(PromotionError::rejected(
                "the finalize preview names a different candidate or stable version",
            ));
        }
        if text(entry, "/sourceSha") != Some(rc.source_sha.as_str()) {
            return Err(PromotionError::rejected(
                "the finalize preview traces the rc to a different source",
            ));
        }
        if expected_next.as_deref() != Some(rc.version.as_str()) {
            return Err(PromotionError::rejected(format!(
                "next is {}, not the approved rc {}",
                shown(expected_next.as_deref().unwrap_or("unassigned")),
                rc.version
            )));
        }
        if expected_latest != rc_plan.prior_latest {
            return Err(PromotionError::rejected(format!(
                "latest moved to {} since the rc plan froze {}",
                shown(expected_latest.as_deref().unwrap_or("unassigned")),
                rc_plan.prior_latest.as_deref().unwrap_or("unassigned")
            )));
        }
        frozen_plan(
            &envelope,
            ReleaseKind::FinalizeRelease,
            rc.source_sha.clone(),
            stable,
            Some(rc.version.clone()),
            "latest",
            expected_next,
            expected_latest,
        )
    }

    /// Confirm a journaled plan. Repeating it after a lost response is the
    /// intended recovery: Release Control answers with the original
    /// operation. A stale plan stops; it is never silently re-previewed.
    pub async fn confirm_plan(&self, plan: &ReleasePlan) -> Step<ConfirmedPlan> {
        let (status, body) = self
            .release_control(
                Method::POST,
                "/operations",
                Some(&json!({
                    "type": plan.kind.as_str(),
                    "planId": plan.plan_id,
                    "planHash": plan.plan_hash,
                    "confirm": true,
                })),
            )
            .await?;
        if status == 409 {
            return Err(PromotionError::rejected(
                "the plan is stale or conflicts (409); a new preview needs a new decision",
            ));
        }
        expect_status(status, 201, "Release Control confirmation")?;
        let operation_id = text(&body, "/operation/id")
            .filter(|id| !id.is_empty())
            .ok_or_else(|| PromotionError::inconclusive("the confirmation named no operation"))?;
        Ok(ConfirmedPlan {
            plan: plan.clone(),
            operation_id: operation_id.to_string(),
        })
    }

    /// One read of the operation. The Harness deployment must be the one the
    /// plan froze; anything else is a divergence, not progress.
    pub async fn read_operation(&self, confirmed: &ConfirmedPlan) -> Step<OperationState> {
        let plan = &confirmed.plan;
        let (status, detail) = self
            .release_control(
                Method::GET,
                &format!("/operations/{}", confirmed.operation_id),
                None,
            )
            .await?;
        expect_status(status, 200, "Release Control operation read")?;
        if text(&detail, "/operation/id") != Some(confirmed.operation_id.as_str()) {
            return Err(PromotionError::rejected(
                "Release Control answered for another operation",
            ));
        }
        let deployments: Vec<&Value> = detail["deployments"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter(|row| text(row, "/worker") == Some(WORKER))
                    .collect()
            })
            .unwrap_or_default();
        let [deployment] = deployments.as_slice() else {
            return Err(PromotionError::rejected(format!(
                "the operation carries {} Harness deployments, not one",
                deployments.len()
            )));
        };
        if text(deployment, "/version") != Some(plan.target_version.as_str())
            || text(deployment, "/sourceSha") != Some(plan.source_sha.as_str())
            || text(deployment, "/channel") != Some(plan.channel.as_str())
            || text(deployment, "/fromVersion") != plan.candidate_version.as_deref()
        {
            return Err(PromotionError::rejected(
                "the Harness deployment differs from the confirmed plan",
            ));
        }
        let status = text(deployment, "/status").unwrap_or_default().to_string();
        Ok(match status.as_str() {
            "converged" => OperationState::Converged(DeploymentEvidence {
                operation_id: confirmed.operation_id.clone(),
                deployment_id: text(deployment, "/id").unwrap_or_default().to_string(),
                version: plan.target_version.clone(),
                channel: plan.channel.clone(),
                from_version: plan.candidate_version.clone(),
                source_sha: plan.source_sha.clone(),
                descriptor_sha256: text(deployment, "/descriptorSha256").map(str::to_string),
                build_run_id: deployment["executions"].as_array().and_then(|runs| {
                    runs.iter()
                        .rev()
                        .find(|run| {
                            text(run, "/conclusion") == Some("success")
                                && run["hasManifest"].as_bool() == Some(true)
                        })
                        .and_then(|run| run["runId"].as_u64())
                }),
            }),
            "blocked" => OperationState::Blocked,
            "discarded" => OperationState::Discarded,
            _ => OperationState::Converging { status },
        })
    }

    /// Read the exact version from the Registry and hash its archive bytes;
    /// the digest the Registry claims must be the one downloaded.
    pub async fn read_artifact(&self, deployment: &DeploymentEvidence) -> Step<ReleaseArtifact> {
        let url = format!(
            "{}/w/{WORKER}/versions",
            self.origins.registry_url.trim_end_matches('/')
        );
        let (status, listing) = self.send(self.http.get(url), "Registry").await?;
        expect_status(status, 200, "Registry versions read")?;
        let row = listing["versions"]
            .as_array()
            .and_then(|rows| {
                rows.iter()
                    .find(|row| text(row, "/version") == Some(deployment.version.as_str()))
            })
            .ok_or_else(|| {
                PromotionError::inconclusive(format!(
                    "the Registry does not list {} yet",
                    deployment.version
                ))
            })?;
        let target = &self.config.artifact_target;
        let binary = &row["binaries"][target.as_str()];
        let archive_url = text(binary, "/url").unwrap_or_default().to_string();
        let claimed = text(binary, "/sha256")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if archive_url.is_empty() || !is_sha256(&claimed) {
            return Err(PromotionError::rejected(format!(
                "{} has no {target} archive with a sha256 in the Registry",
                deployment.version
            )));
        }
        let archive = self.download_archive(&archive_url).await?;
        let downloaded = format!("{:x}", Sha256::digest(&archive));
        if downloaded != claimed {
            return Err(PromotionError::rejected(format!(
                "the {target} archive of {} hashes to {downloaded}, the Registry claims {claimed}",
                deployment.version
            )));
        }
        Ok(ReleaseArtifact {
            version: deployment.version.clone(),
            tags: row["tags"]
                .as_array()
                .map(|tags| {
                    tags.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            target: target.clone(),
            archive_url,
            archive_sha256: downloaded,
            binary_sha256: tokio::task::spawn_blocking(move || archive_binary_sha256(&archive))
                .await
                .map_err(|_| PromotionError::unavailable("the archive read task failed"))??,
            source_sha: deployment.source_sha.clone(),
            operation_id: deployment.operation_id.clone(),
            deployment_id: deployment.deployment_id.clone(),
            build_run_id: deployment.build_run_id,
        })
    }

    // ── Compose ────────────────────────────────────────────────────────────────

    /// What the configured target runs right now, with the digest of the
    /// executable behind its pid when the target is local Linux.
    pub async fn observe_target(&self) -> Step<TargetObservation> {
        let target = &self.config.target;
        let status = self
            .compose(
                COMPOSE_STATUS,
                json!({ "namespace": target.namespace, "file": target.compose_file }),
            )
            .await?;
        let namespace = text(&status, "/namespace").unwrap_or_default().to_string();
        if namespace != target.namespace {
            return Err(PromotionError::rejected(
                "Compose answered for another namespace than the configured target",
            ));
        }
        let file = text(&status, "/file").unwrap_or_default().to_string();
        if !same_path(&file, &target.compose_file) {
            return Err(PromotionError::rejected(
                "Compose answered for another compose file than the configured one",
            ));
        }
        let row = status["containers"]
            .as_array()
            .and_then(|rows| {
                rows.iter()
                    .find(|row| text(row, "/container") == Some(WORKER))
            })
            .ok_or_else(|| {
                PromotionError::rejected(format!("{WORKER} is not declared on the target"))
            })?;
        let state = text(row, "/state").unwrap_or_default().to_string();
        let supervisor_pid = row["pid"].as_u64().and_then(|pid| u32::try_from(pid).ok());
        let (pid, binary_sha256, binary_unknown_reason) = match (state.as_str(), supervisor_pid) {
            _ if !target.is_local_linux() => (
                None,
                None,
                Some("the target is not a local Linux engine".to_string()),
            ),
            ("ready", Some(supervisor)) => {
                match tokio::task::spawn_blocking(move || capture_harness(supervisor)).await {
                    Ok(Ok((pid, digest))) => (Some(pid), Some(digest), None),
                    Ok(Err(reason)) => (None, None, Some(reason)),
                    Err(_) => (None, None, Some("the capture task failed".to_string())),
                }
            }
            _ => (None, None, Some(format!("{WORKER} is {}", shown(&state)))),
        };
        Ok(TargetObservation {
            file,
            namespace,
            container: WORKER.to_string(),
            state,
            version: text(row, "/version").map(str::to_string),
            supervisor_pid,
            pid,
            binary_sha256,
            binary_unknown_reason,
        })
    }

    /// Read one Compose operation for recovery. Read-only: it never submits
    /// or retries the pin, so an operation cancelled before its add stays
    /// cancelled. `Succeeded` is not activation either; prove that with
    /// [`Self::confirm_running`].
    pub async fn read_compose_operation(&self, operation_id: &str) -> Step<ComposeOperationState> {
        let Some(snapshot) = self.operation_snapshot(operation_id).await? else {
            return Ok(ComposeOperationState::Unknown);
        };
        Ok(match text(&snapshot, "/status") {
            Some("running") => ComposeOperationState::Running,
            Some("succeeded") => ComposeOperationState::Succeeded,
            Some("failed") => ComposeOperationState::Failed,
            Some("cancelled") => ComposeOperationState::Cancelled,
            _ => ComposeOperationState::Unknown,
        })
    }

    /// Pin the stable version on a target that runs the validated prior.
    /// A target already on the candidate sends nothing.
    pub async fn request_activation(
        &self,
        prior: &ValidatedArtifact,
        candidate: &ValidatedArtifact,
        operation_id: &str,
    ) -> Step<ComposeRequest> {
        self.request_pin(prior, candidate, candidate, operation_id)
            .await
    }

    /// Pin the validated prior back on a target that runs the candidate. A
    /// target already on the prior sends nothing.
    pub async fn request_rollback(
        &self,
        prior: &ValidatedArtifact,
        candidate: &ValidatedArtifact,
        operation_id: &str,
    ) -> Step<ComposeRequest> {
        self.request_pin(prior, candidate, prior, operation_id)
            .await
    }

    async fn request_pin(
        &self,
        prior: &ValidatedArtifact,
        candidate: &ValidatedArtifact,
        destination: &ValidatedArtifact,
        operation_id: &str,
    ) -> Step<ComposeRequest> {
        for artifact in [prior, candidate] {
            if !is_sha256(&artifact.binary_sha256) {
                return Err(PromotionError::rejected(format!(
                    "{} has no validated executable digest",
                    artifact.version
                )));
            }
        }
        let requested = ComposeRequest {
            operation_id: operation_id.to_string(),
            version: destination.version.clone(),
            accepted: true,
        };
        // A restart lands here with the operation possibly already running
        // or done: reconcile it before sending anything.
        if let Some(snapshot) = self.operation_snapshot(operation_id).await? {
            return match text(&snapshot, "/status") {
                Some("running" | "succeeded") => Ok(requested),
                other => Err(PromotionError::rejected(format!(
                    "Compose operation {operation_id} ended {}",
                    other.unwrap_or("unknown")
                ))),
            };
        }
        let observed = self.observe_target().await?;
        let source = if destination == candidate {
            prior
        } else {
            candidate
        };
        match classify_target(&observed, prior, candidate) {
            TargetIs::Unknown { .. }
                if destination == prior
                    && matches!(observed.state.as_str(), "failed" | "stopped")
                    && [prior.version.as_str(), candidate.version.as_str()]
                        .contains(&observed.version.as_deref().unwrap_or_default()) => {}
            TargetIs::Unknown { reason } => return Err(PromotionError::inconclusive(reason)),
            TargetIs::Other { reason } => return Err(PromotionError::rejected(reason)),
            TargetIs::Prior if destination == prior => {
                return Ok(ComposeRequest {
                    accepted: false,
                    ..requested
                })
            }
            TargetIs::Candidate if destination == candidate => {
                return Ok(ComposeRequest {
                    accepted: false,
                    ..requested
                })
            }
            _ if observed.version.as_deref() == Some(source.version.as_str()) => {}
            _ => {
                return Err(PromotionError::rejected(
                    "the target does not run the expected source version",
                ))
            }
        }
        let target = &self.config.target;
        let accepted = self
            .compose(
                COMPOSE_ADD,
                json!({
                    "namespace": target.namespace,
                    "file": target.compose_file,
                    "workers": [{ "worker": format!("package://{WORKER}"), "version": destination.version }],
                    "operation_id": operation_id,
                }),
            )
            .await?;
        if text(&accepted, "/operation_id") != Some(operation_id) {
            return Err(PromotionError::inconclusive(
                "Compose accepted the pin under another operation id",
            ));
        }
        Ok(requested)
    }

    /// Whether the target now runs `expected`: the operation is over, the
    /// container is ready on that version, and the executable behind its pid
    /// hashes to the validated digest. A request alone is pending.
    pub async fn confirm_running(
        &self,
        expected: &ValidatedArtifact,
        operation_id: &str,
    ) -> Step<Confirmation> {
        let pending =
            |reason: String| -> Step<Confirmation> { Ok(Confirmation::Pending { reason }) };
        if !self.config.target.is_local_linux() {
            return Err(PromotionError::unavailable(
                "the target is not a local Linux engine; its executable cannot be proved",
            ));
        }
        if let Some(snapshot) = self.operation_snapshot(operation_id).await? {
            match text(&snapshot, "/status") {
                Some("succeeded") => {}
                Some("running") => {
                    return pending(format!("Compose operation {operation_id} is running"))
                }
                other => {
                    return Err(PromotionError::rejected(format!(
                        "Compose operation {operation_id} ended {}",
                        other.unwrap_or("unknown")
                    )))
                }
            }
        }
        let observed = self.observe_target().await?;
        if observed.state != "ready" {
            return pending(format!(
                "{} is {}",
                observed.container,
                shown(&observed.state)
            ));
        }
        if observed.version.as_deref() != Some(expected.version.as_str()) {
            return pending(format!(
                "{} runs {}, not {}",
                observed.container,
                shown(observed.version.as_deref().unwrap_or("unpinned")),
                expected.version
            ));
        }
        let (Some(binary), Some(pid), Some(supervisor_pid)) = (
            observed.binary_sha256.clone(),
            observed.pid,
            observed.supervisor_pid,
        ) else {
            return pending(
                observed
                    .binary_unknown_reason
                    .unwrap_or_else(|| "the running executable is unknown".into()),
            );
        };
        if binary != expected.binary_sha256 {
            return Err(PromotionError::rejected(format!(
                "{} {} runs executable {binary}, not the validated {}",
                observed.container, expected.version, expected.binary_sha256
            )));
        }
        Ok(Confirmation::Confirmed(RunningEvidence {
            operation_id: operation_id.to_string(),
            version: expected.version.clone(),
            supervisor_pid,
            pid,
            binary_sha256: binary,
            file: observed.file,
            namespace: observed.namespace,
        }))
    }

    async fn operation_snapshot(&self, operation_id: &str) -> Step<Option<Value>> {
        let target = &self.config.target;
        match self
            .compose_call(
                COMPOSE_OPERATION,
                json!({ "namespace": target.namespace, "progress_operation_id": operation_id }),
            )
            .await
        {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(ComposeFailure::UnknownOperation) => Ok(None),
            Err(ComposeFailure::Stop(error)) => Err(error),
        }
    }

    async fn compose(&self, function: &'static str, payload: Value) -> Step<Value> {
        self.compose_call(function, payload)
            .await
            .map_err(|failure| match failure {
                ComposeFailure::Stop(error) => error,
                ComposeFailure::UnknownOperation => {
                    PromotionError::rejected(format!("{function} refused the call"))
                }
            })
    }

    /// One Compose call on the target Engine, in the target namespace. Only
    /// `compose::add`, `compose::operation` and `compose::status` are used.
    /// The daemon's own error text is read to recognise an unknown operation
    /// and is never kept.
    async fn compose_call(
        &self,
        function: &'static str,
        payload: Value,
    ) -> Result<Value, ComposeFailure> {
        let request = TriggerRequest {
            function_id: function.into(),
            payload,
            action: None,
            timeout_ms: Some(COMPOSE_TIMEOUT_MS),
        }
        .namespace(self.config.target.namespace.clone());
        let wait = Duration::from_millis(COMPOSE_TIMEOUT_MS + 5_000);
        match tokio::time::timeout(wait, self.target_engine.trigger(request)).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(iii_sdk::errors::Error::Remote { message, .. }))
                if function == COMPOSE_OPERATION
                    && message.contains("unknown compose operation") =>
            {
                Err(ComposeFailure::UnknownOperation)
            }
            Ok(Err(iii_sdk::errors::Error::Remote { .. })) => Err(ComposeFailure::Stop(
                PromotionError::rejected(format!("{function} refused the call")),
            )),
            Ok(Err(iii_sdk::errors::Error::Timeout)) | Err(_) => Err(ComposeFailure::Stop(
                PromotionError::inconclusive(format!("{function} did not answer")),
            )),
            Ok(Err(_)) => Err(ComposeFailure::Stop(PromotionError::unavailable(format!(
                "{function} could not reach the target engine"
            )))),
        }
    }

    // ── transport ───────────────────────────────────────────────────────────────

    async fn release_control(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Step<(u16, Value)> {
        let session = secret(RELEASE_CONTROL_SESSION_ENV)?;
        let url = format!(
            "{}{path}",
            self.origins.release_control_url.trim_end_matches('/')
        );
        let mut request = self
            .http
            .request(method, url)
            .header(COOKIE, format!("{SESSION_COOKIE}={session}"));
        if let Some(body) = body {
            request = request
                .header(CONTENT_TYPE, "application/json")
                .body(body.to_string());
        }
        let (status, value) = self.send(request, "Release Control").await?;
        if status == 401 {
            return Err(PromotionError::unavailable(
                "the Release Control session is missing or expired",
            ));
        }
        Ok((status, value))
    }

    async fn github(&self, method: Method, path: &str, body: Option<&Value>) -> Step<(u16, Value)> {
        let token = secret(GITHUB_TOKEN_ENV)?;
        let url = format!(
            "{}/repos/{REPOSITORY}{path}",
            self.origins.github_api_url.trim_end_matches('/')
        );
        let mut request = self
            .http
            .request(method, url)
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(body) = body {
            request = request
                .header(CONTENT_TYPE, "application/json")
                .body(body.to_string());
        }
        let (status, value) = self.send(request, "GitHub").await?;
        if status == 401 {
            return Err(PromotionError::unavailable(
                "the GitHub token is missing or expired",
            ));
        }
        Ok((status, value))
    }

    /// Send and read a JSON envelope. A request that may have reached the
    /// service and lost its answer is inconclusive, not unavailable.
    async fn send(&self, request: reqwest::RequestBuilder, service: &str) -> Step<(u16, Value)> {
        let response = request.send().await.map_err(|error| lost(service, error))?;
        let status = response.status().as_u16();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| lost(service, error))?;
        if status >= 500 {
            return Err(PromotionError::unavailable(format!(
                "{service} answered {status}"
            )));
        }
        Ok((
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        ))
    }

    /// The archive bytes, bounded. Kept in memory only: nothing is written
    /// to or extracted on the filesystem.
    async fn download_archive(&self, url: &str) -> Step<Vec<u8>> {
        let mut response = self
            .http
            .get(url)
            .timeout(ARCHIVE_TIMEOUT)
            .send()
            .await
            .map_err(|error| lost("archive download", error))?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err(PromotionError::unavailable(format!(
                "archive download answered {status}"
            )));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| lost("archive download", error))?
        {
            if bytes.len() + chunk.len() > ARCHIVE_MAX_BYTES {
                return Err(PromotionError::rejected(
                    "the archive is larger than any Harness release",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn secret(name: &str) -> Step<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            PromotionError::unavailable(format!("credential variable {name} is not set"))
        })
}

fn lost(service: &str, error: reqwest::Error) -> PromotionError {
    // Only the kind: a transport error's text can carry a URL or a peer's
    // words, neither of which belongs in a journal.
    if error.is_connect() || error.is_builder() {
        PromotionError::unavailable(format!("{service} is unreachable"))
    } else {
        PromotionError::inconclusive(format!("{service} response was lost"))
    }
}

/// A status other than the expected one. Only this module's own words and
/// the status go into the reason: a remote `code`, `error` or `message` can
/// carry anything, secrets included.
fn expect_status(status: u16, expected: u16, what: &str) -> Step<()> {
    if status == expected {
        return Ok(());
    }
    Err(PromotionError::rejected(format!(
        "{what} answered {status}"
    )))
}

fn single_entry(preview: &Value) -> Step<&Value> {
    let entries = preview["plan"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let blocking = entries
        .iter()
        .filter(|entry| !entry["blocking"].is_null())
        .count();
    if preview["blocked"].as_bool() != Some(false) || blocking > 0 {
        return Err(PromotionError::rejected(format!(
            "Release Control blocked the plan ({blocking} blocking entries); its reasons stay in \
             Release Control"
        )));
    }
    match entries {
        [entry] if text(entry, "/worker") == Some(WORKER) => Ok(entry),
        _ => Err(PromotionError::rejected(
            "the plan does not contain exactly the Harness entry",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn frozen_plan(
    envelope: &Value,
    kind: ReleaseKind,
    source_sha: String,
    target_version: String,
    candidate_version: Option<String>,
    channel: &str,
    prior_next: Option<String>,
    prior_latest: Option<String>,
) -> Step<ReleasePlan> {
    let plan_id = text(envelope, "/planId").unwrap_or_default();
    let plan_hash = text(envelope, "/planHash").unwrap_or_default();
    if plan_id.is_empty() || !is_sha256(plan_hash) {
        return Err(PromotionError::inconclusive(
            "the preview carried no plan id and hash",
        ));
    }
    Ok(ReleasePlan {
        kind,
        plan_id: plan_id.to_string(),
        plan_hash: plan_hash.to_string(),
        expires_at: text(envelope, "/expiresAt").unwrap_or_default().to_string(),
        source_sha,
        target_version,
        candidate_version,
        channel: channel.to_string(),
        prior_next,
        prior_latest,
        preview_sha256: format!(
            "{:x}",
            Sha256::digest(envelope["preview"].to_string().as_bytes())
        ),
    })
}

fn pull_evidence(pull: &Value, candidate_sha: &str, created: bool) -> Step<PullRequestEvidence> {
    let head_sha = text(pull, "/head/sha").unwrap_or_default();
    if head_sha != candidate_sha {
        return Err(PromotionError::rejected(format!(
            "the pull request head is {}, not the approved candidate",
            shown(head_sha)
        )));
    }
    Ok(PullRequestEvidence {
        number: pull["number"]
            .as_u64()
            .ok_or_else(|| PromotionError::inconclusive("the pull request has no number"))?,
        url: text(pull, "/html_url").unwrap_or_default().to_string(),
        head_ref: text(pull, "/head/ref").unwrap_or_default().to_string(),
        head_sha: head_sha.to_string(),
        created,
    })
}

/// Find the one Harness process at or under the Compose-supervised pid and
/// hash its executable. Compose may supervise a wrapper (`sh -c ...`), so
/// the supervised pid is never hashed as Harness unless its own executable
/// is Harness. Each process is identified by its start time, read in the
/// same `/proc/<pid>/stat` snapshot as its parent: a child must name the
/// walked parent, the hashed process must keep its start time, and after
/// the bytes are read its parent chain must still lead to the unchanged
/// supervisor. A reused or detached pid is reported, never hashed as
/// Harness. None or several Harness processes is unknown.
fn capture_harness(supervisor: u32) -> Result<(u32, String), String> {
    const MAX_PROCESSES: usize = 4096;
    let (supervisor_token, _) = stat(supervisor)?;
    let mut found = Vec::new();
    // (pid, start time, parent pid as walked)
    let mut queue = vec![(supervisor, supervisor_token, None::<u32>)];
    let mut seen = std::collections::HashSet::new();
    while let Some((pid, token, walked_parent)) = queue.pop() {
        if seen.len() >= MAX_PROCESSES {
            return Err("the supervisor's process tree is too large to walk".into());
        }
        if !seen.insert(pid) {
            continue;
        }
        if is_harness(pid) {
            found.push((pid, token, walked_parent));
        }
        for child in children(pid) {
            // A child that exited is simply not part of the tree; one whose
            // parent is no longer this process was reused or detached.
            let Ok((child_token, parent)) = stat(child) else {
                continue;
            };
            if parent == pid {
                queue.push((child, child_token, Some(pid)));
            }
        }
    }
    if stat(supervisor)?.0 != supervisor_token {
        return Err("the supervised process changed during the capture".into());
    }
    let (pid, token, walked_parent) = match found.as_slice() {
        [one] => *one,
        [] => {
            return Err(format!(
                "no {WORKER} process runs under supervisor {supervisor}"
            ))
        }
        many => {
            return Err(format!(
                "{} {WORKER} processes run under supervisor {supervisor}",
                many.len()
            ))
        }
    };
    let changed = || format!("{WORKER} process {pid} changed while it was hashed");
    let (before, parent) = stat(pid).map_err(|_| changed())?;
    if before != token || (walked_parent.is_some() && Some(parent) != walked_parent) {
        return Err(changed());
    }
    let digest = (|| -> std::io::Result<String> {
        let mut file = std::fs::File::open(format!("/proc/{pid}/exe"))?;
        let mut digest = Sha256::new();
        std::io::copy(&mut file, &mut digest)?;
        Ok(format!("{:x}", digest.finalize()))
    })()
    .map_err(|_| format!("/proc/{pid}/exe could not be read"))?;
    if stat(pid).map_err(|_| changed())?.0 != token || !is_harness(pid) {
        return Err(changed());
    }
    // After the read: the hashed process must still descend from the same
    // supervisor, following its actual parents.
    let mut current = pid;
    let mut hops = 0;
    while current != supervisor {
        hops += 1;
        if hops > MAX_PROCESSES {
            return Err(changed());
        }
        let (_, parent) = stat(current).map_err(|_| changed())?;
        if parent <= 1 {
            return Err(format!(
                "{WORKER} process {pid} no longer runs under supervisor {supervisor}"
            ));
        }
        current = parent;
    }
    if stat(supervisor)?.0 != supervisor_token {
        return Err("the supervised process changed during the capture".into());
    }
    Ok((pid, digest))
}

/// Whether the executable behind a pid is named `harness`, including one
/// replaced on disk since it started.
fn is_harness(pid: u32) -> bool {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .and_then(|path| {
            let path = path.to_string_lossy().to_string();
            path.rsplit('/').next().map(str::to_string)
        })
        .is_some_and(|name| name == WORKER || name == format!("{WORKER} (deleted)"))
}

/// Direct children of every thread of a process; empty once it is gone.
fn children(pid: u32) -> Vec<u32> {
    let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else {
        return Vec::new();
    };
    tasks
        .flatten()
        .filter_map(|task| std::fs::read_to_string(task.path().join("children")).ok())
        .flat_map(|list| {
            list.split_whitespace()
                .filter_map(|child| child.parse().ok())
                .collect::<Vec<u32>>()
        })
        .collect()
}

/// `(start time, parent pid)` from one read of `/proc/<pid>/stat`: fields 22
/// and 4, read after the parenthesised name so a name with spaces or
/// parentheses cannot shift them.
fn stat(pid: u32) -> Result<(u64, u32), String> {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            let (_, rest) = stat.rsplit_once(')')?;
            let fields: Vec<&str> = rest.split_whitespace().collect();
            Some((fields.get(19)?.parse().ok()?, fields.get(1)?.parse().ok()?))
        })
        .ok_or_else(|| format!("process {pid} is gone"))
}

/// sha256 of the archive's one regular-file entry named `harness` at its
/// root (`harness` or `./harness`), streamed from the gzip tar in memory.
/// Another entry type under that name, a second one, or none is refused.
/// Other entries (runtime companions) are skipped unread.
fn archive_binary_sha256(archive: &[u8]) -> Step<String> {
    let unreadable = || PromotionError::rejected("the release archive is not a readable tar.gz");
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let mut digest = None;
    for entry in tar.entries().map_err(|_| unreadable())? {
        let mut entry = entry.map_err(|_| unreadable())?;
        let named_harness = {
            let path = entry.path().map_err(|_| unreadable())?;
            let path = path.to_string_lossy();
            path == WORKER || path == format!("./{WORKER}")
        };
        if !named_harness {
            continue;
        }
        if entry.header().entry_type() != tar::EntryType::Regular {
            return Err(PromotionError::rejected(format!(
                "the archive's {WORKER} entry is not a regular file"
            )));
        }
        if digest.is_some() {
            return Err(PromotionError::rejected(format!(
                "the archive holds more than one {WORKER} entry"
            )));
        }
        let mut hasher = Sha256::new();
        std::io::copy(&mut entry, &mut hasher).map_err(|_| unreadable())?;
        digest = Some(format!("{:x}", hasher.finalize()));
    }
    digest.ok_or_else(|| {
        PromotionError::rejected(format!("the archive holds no {WORKER} executable"))
    })
}

fn same_path(reported: &str, configured: &str) -> bool {
    let canonical = |path: &str| std::fs::canonicalize(path).ok();
    reported == configured
        || matches!((canonical(reported), canonical(configured)), (Some(a), Some(b)) if a == b)
}

/// A remote value fit for a reason: identifiers and versions pass, anything
/// else is withheld.
fn shown(value: &str) -> &str {
    let plain = !value.is_empty()
        && value.len() <= 80
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'));
    if plain {
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

fn is_sha256(value: &str) -> bool {
    value.len() == 64
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
