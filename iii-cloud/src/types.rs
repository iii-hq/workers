//! The wire types of spec §11. The golden schemas in `tests/golden/schemas/`
//! are their executable form; the UI copies them into `ui/src/types.ts`.
//! The store (`state.json`) persists the same records.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::error::WorkerError;

// ---------------------------------------------------------------- requests

/// `iii-cloud::connect` input.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum ConnectRequest {
    /// Sign in with a code typed in the browser (the main path).
    Browser,
    /// Use an environment-scoped API key saved in the vault.
    ApiKey {
        /// The vault secret that holds the key. Never the key itself.
        secret: String,
        /// Only for an older cloud without `GET /keys/self`: the key's team.
        #[serde(default)]
        team_id: Option<String>,
        /// Only for an older cloud without `GET /keys/self`: the key's environment.
        #[serde(default)]
        environment_id: Option<String>,
    },
}

/// `iii-cloud::status` input.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct StatusRequest {
    /// Re-read the checkout, the pull request, the variables and the deployments first.
    #[serde(default)]
    pub refresh: Option<bool>,
}

/// `iii-cloud::project::resolve` input.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResolveRequest {
    /// A local folder inside the repository.
    pub path: String,
    /// Look only in this team.
    #[serde(default)]
    pub team_id: Option<String>,
    /// Several cloud projects use the repository: take this one.
    #[serde(default)]
    pub cloud_project_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMode {
    Worktree,
    Checkout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UseFrom {
    Environment,
    Checkout,
}

/// `iii-cloud::project::use` input.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UseRequest {
    pub path: String,
    /// The environment; or only `cloud_project_id`, for that project's default one.
    #[serde(default)]
    pub environment_id: Option<String>,
    /// `worktree` (recommended) or `checkout`.
    pub mode: WorkspaceMode,
    /// Several cloud projects use the repository: take this one.
    #[serde(default)]
    pub cloud_project_id: Option<String>,
    /// Where the worktree's commits come from. `environment` (default): the
    /// environment's branch. `checkout`: the person's own HEAD with the
    /// environment's head: a fast-forward when one contains the other, else a
    /// merge commit made in the worktree (never in the checkout); a conflict
    /// leaves the worktree as it was (`MERGE_CONFLICT`, the files in the
    /// message). Only commits move: nothing uncommitted is copied. Never for
    /// production, which goes through a pull request.
    #[serde(default)]
    pub from: Option<UseFrom>,
    /// With `from: checkout`: take this branch (`main`, `origin/feature-x`,
    /// one of `checkout.branches`) instead of the checkout's HEAD, by the same
    /// rule.
    #[serde(default)]
    pub source: Option<String>,
}

/// `iii-cloud::project::create` input: import this repository as a new cloud project.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ProjectCreateRequest {
    /// A local folder inside the repository.
    pub path: String,
    pub team_id: String,
    /// One of the repository's choosable compose files, relative to its root.
    pub manifest: String,
    /// Lowercase letters, digits and hyphens, at most 30.
    pub environment_name: String,
    /// The first environment is production (default true).
    #[serde(default)]
    pub production: Option<bool>,
}

/// `iii-cloud::project::create_environment` input.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct EnvironmentCreateRequest {
    pub cloud_project_id: String,
    pub name: String,
    /// Default: the repository's default branch.
    #[serde(default)]
    pub branch: Option<String>,
    /// Default: the selected environment's compose file.
    #[serde(default)]
    pub manifest: Option<String>,
    /// Default: the project's production environment, so secrets and settings carry over.
    #[serde(default)]
    pub copy_from: Option<String>,
    #[serde(default)]
    pub production: Option<bool>,
    /// Copy nothing: no `copy_from` default. Not with `copy_from`.
    #[serde(default)]
    pub start_empty: Option<bool>,
}

/// The answer of `project::create` and `project::create_environment`: the cloud
/// projects read fresh, the new ids, and what is selected now.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateResponse {
    pub cloud_projects: Vec<CloudProjectInfo>,
    pub project_id: Option<String>,
    pub environment_id: Option<String>,
    /// The project as selected after `project::use`; while another project
    /// with a workspace is selected, the new one as `Status.candidate`, not selected.
    pub project: Project,
}

/// `iii-cloud::api` input.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ApiRequest {
    pub method: HttpMethod,
    /// Relative to `/api/v1`; starts with `/`, no `..`, `//` or scheme.
    pub path: String,
    #[serde(default)]
    pub query: Option<Value>,
    #[serde(default)]
    pub body: Option<Value>,
    #[serde(default)]
    pub if_match: Option<String>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

/// `iii-cloud::api` output: the cloud's answer, passed through.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ApiResponse {
    pub status: u16,
    pub body: Value,
}

/// `iii-cloud::check` input.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct CheckRequest {
    /// An agent works on its own hand-off: use that one's environment and worktree.
    #[serde(default)]
    pub handoff_id: Option<String>,
    /// Default: the workspace's `HEAD`.
    #[serde(default)]
    pub commit: Option<String>,
    /// Also run the app here the way the cloud runs it.
    #[serde(default)]
    pub run: Option<bool>,
    /// Check another `worker-compose*.yaml` of the repository (from its root)
    /// instead of the environment's. Nothing changes in iii cloud: the
    /// environment switches to it only when the person deploys this check.
    /// Default: the hand-off's file, else the environment's.
    #[serde(default)]
    pub manifest: Option<String>,
    /// Push the workspace's fix branch first (`--force-with-lease`), so the
    /// cloud can plan its head. `PUSH_REFUSED` for a branch an environment
    /// tracks or that is not an `iii-cloud/` fix branch.
    #[serde(default)]
    pub push: Option<bool>,
    /// With `run`: run again even when a passed run of the same commit, compose
    /// file, engine and run environment exists.
    #[serde(default)]
    pub force: Option<bool>,
    /// The page's own: a check the person started on the page. When the hand-off's agent
    /// waits on a wake, it hears what is left for it. Agents never send it.
    #[serde(default)]
    pub from_page: bool,
}

/// `iii-cloud::check::get` input.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckGetRequest {
    /// A check's id: `status.checks[].id`, a deploy's `check_id`.
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SecretMode {
    #[default]
    Ask,
    NotNeeded,
    Decide,
}

/// `iii-cloud::secret` input.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SecretAsk {
    #[serde(default)]
    pub handoff_id: Option<String>,
    /// `^[A-Z_][A-Z0-9_]*$`, not starting with `III_`.
    pub name: String,
    /// What the person needs to know to give the value.
    pub description: String,
    #[serde(default)]
    pub mode: Option<SecretMode>,
    /// The person chose a database outside iii cloud: only then may a database
    /// URL be asked for.
    #[serde(default)]
    pub external: Option<bool>,
}

/// `iii-cloud::secret::provide` input. Page only.
#[derive(Deserialize, JsonSchema)]
pub struct ProvideRequest {
    pub request_id: String,
    /// The secret value. Never logged, never returned.
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub leave_empty: Option<bool>,
}

/// `iii-cloud::deploy` input.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeployRequest {
    #[serde(default)]
    pub handoff_id: Option<String>,
    pub check_id: String,
    /// Merge this pull request first (or resume after its merge).
    #[serde(default)]
    pub pr: Option<u64>,
    /// Confirm this waiting deployment (a merge made directly on GitHub); its
    /// plan digest must match the check's.
    #[serde(default)]
    pub deployment_id: Option<String>,
}

/// `iii-cloud::watch` input.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct WatchRequest {
    #[serde(default)]
    pub handoff_id: Option<String>,
    /// Default: the newest deployment of the selected environment.
    #[serde(default)]
    pub deployment_id: Option<String>,
}

/// `iii-cloud::handoff` input: the page hands work to the agent.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct HandoffRequest {
    /// Required for every kind but `import`.
    #[serde(default)]
    pub check_id: Option<String>,
    pub kind: HandoffKind,
    /// For `deploy_fix`: the deployment that failed.
    #[serde(default)]
    pub deployment_id: Option<String>,
    /// The plain chat text the page would post. With it, a follow-up goes into
    /// the agent's existing chat: the worker sends it itself.
    #[serde(default)]
    pub text: Option<String>,
    /// true when sent from the add-project steps: brief the folder being added.
    #[serde(default)]
    pub candidate: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandoffKind {
    Ready,
    DeployFix,
    /// The project is not in iii cloud yet.
    Import,
    PullRequest,
}

/// `iii-cloud::handoff` output.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HandoffResponse {
    pub id: String,
    /// A short line the page puts at the top of the prompt; the worker finds
    /// the agent's session by it.
    pub marker: String,
    /// The existing chat the worker sent the hand-off into.
    pub session_id: Option<String>,
    /// False: nothing was sent; the page starts a new chat with the marker.
    pub delivered: bool,
    /// True: an equal hand-off was already open (a double click), and this is it.
    #[serde(default)]
    pub reused: bool,
    /// With `reused`: the worker sent the open hand-off's agent on (the same "continue" as after a
    /// person's check: the newest check's blocking problems of the agent's, and its failed run).
    /// False: nothing was sent (it works, asks the person something, or has nothing open).
    /// Absent for a new hand-off.
    #[serde(default)]
    pub sent: Option<bool>,
}

/// `iii-cloud::handoff::stop` input.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct HandoffStopRequest {
    pub id: String,
}

/// `iii-cloud::handoff::worktree` input.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct HandoffWorktreeRequest {
    /// The hand-off (default: the selected environment's open one). `id` works too.
    #[serde(default, alias = "id")]
    pub handoff_id: Option<String>,
}

/// The worktree a hand-off works in: never the person's checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HandoffWorktree {
    pub path: String,
    /// The fix branch, `iii-cloud/ready-<env>` (or `-2`, ...).
    pub branch: String,
    /// What the worktree started from: the person's commit it took, else the
    /// environment's head.
    pub base: Option<BriefBase>,
    /// This call made the worktree; false: it was there.
    pub created: bool,
    /// The person's branch (or the check's `source`) this call merged in on
    /// top of the environment's branch; null when nothing came in.
    pub merged_from: Option<String>,
    /// The person's commits conflict with the environment's branch: the
    /// worktree is at the base, and this says what to merge.
    pub conflict: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HandoffStopResponse {
    pub id: String,
    /// The harness accepted the stop of the session's running turn.
    pub stopping: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentPhase {
    Starting,
    Working,
    WaitingForYou,
    /// The turn ended on a wake the agent armed (an observer that notifies its
    /// chat, unfired and unexpired): it waits on an event, not on the person.
    Waiting,
    Stalled,
    Done,
    Stopped,
}

/// What the agent is doing for a hand-off, as the worker sees it. Descriptions
/// only: no message body, tool input or result is ever copied here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentState {
    pub handoff_id: String,
    /// What the hand-off is for, so the page can place its row.
    #[serde(default)]
    pub kind: Option<HandoffKind>,
    pub session_id: Option<String>,
    pub state: AgentPhase,
    pub started_at: String,
    pub last_activity_at: Option<String>,
    /// Function calls the session has made.
    pub steps: u32,
    /// The last call's description, plain text, at most 120 characters.
    pub current: Option<String>,
    /// The open question's text, when the agent waits for an answer.
    pub question: Option<String>,
    /// Ids of the newest check's problems the agent is working on now (the
    /// page shows a spinner on them). Cleared when they leave a newer check
    /// or the hand-off ends.
    #[serde(default)]
    pub working_on: Vec<String>,
    /// One plain line from the agent about what it is doing, at most 120 characters.
    #[serde(default)]
    pub note: Option<String>,
    /// What the agent waits for when `state` is `waiting` ("the run here to end").
    #[serde(default)]
    pub waiting_for: Option<String>,
    /// Where `question` comes from: a real ask from the agent's session, or text this
    /// worker made from the check's open problems. Absent when there is no question.
    #[serde(default)]
    pub question_source: Option<QuestionSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuestionSource {
    /// An ask the agent made in its session.
    Agent,
    /// The worker's own words about what the check leaves open.
    Check,
}

/// `iii-cloud::handoff::progress` input: the agent says what it starts on.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct HandoffProgressRequest {
    /// The hand-off this progress is for, by its id or the 8 characters of the
    /// marker (default: the selected environment's open one).
    #[serde(default)]
    pub handoff_id: Option<String>,
    /// Problem ids, or codes (a code stands for every open problem of that code).
    pub working_on: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HandoffProgressResponse {
    /// The problem ids the names resolved to in the newest check.
    pub working_on: Vec<String>,
    pub note: Option<String>,
}

/// A hand-off the worker follows. One open per environment; a new one replaces it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Handoff {
    pub id: String,
    pub marker: String,
    pub environment_id: String,
    pub check_id: Option<String>,
    pub kind: HandoffKind,
    pub deployment_id: Option<String>,
    /// Milliseconds since epoch: only sessions created after it can be ours.
    pub asked_at_ms: i64,
    pub agent: AgentState,
    /// The fast check after the agent finished has run.
    #[serde(default)]
    pub checked: bool,
    /// Text to send into the chat once it is found (a double click before it started).
    #[serde(default)]
    pub pending_text: Option<String>,
    /// Where this hand-off works: the agent's tools follow it, not the global selection.
    #[serde(default)]
    pub workspace_path: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub team_id: String,
    #[serde(default)]
    pub env: Option<EnvironmentInfo>,
    /// Asked from the add-project steps: an import's brief is of `State.candidate`.
    #[serde(default)]
    pub candidate: bool,
    /// What the agent must do first in its worktree (the person's commits that
    /// did not come in), from `handoff::worktree`.
    #[serde(default)]
    pub worktree_note: Option<String>,
    /// "Continue" messages the worker posted in the chat (at most 3).
    #[serde(default)]
    pub continues: u32,
    /// When the worker last tried a "continue" (sent or refused): until the
    /// session works again, or a stall passes, it tries no other.
    #[serde(default)]
    pub continue_at_ms: Option<i64>,
    /// The chat refused the last "continue" (so the state after a restart is the same).
    #[serde(default)]
    pub continue_refused: bool,
    /// Fast checks that failed in a row (reset by one that worked).
    #[serde(default)]
    pub check_failures: u32,
    /// The run the "it is waiting for a container" continue went out for: told once
    /// per run, never counted against the cap, and judged no more while it goes on.
    #[serde(default)]
    pub waited_run: Option<String>,
    /// A check the person started on this hand-off's worktree ended while the agent
    /// waited on a wake: the poll judges it once (`person_check_told` is the last
    /// one it did), and sends the agent what it owns.
    #[serde(default)]
    pub person_check: Option<String>,
    #[serde(default)]
    pub person_check_told: Option<String>,
}

impl Handoff {
    /// `id` is this hand-off's id, or the 8 characters its marker line carries.
    pub fn is(&self, id: &str) -> bool {
        self.id == id || (id.len() == 8 && self.id.starts_with(id))
    }
}

/// `iii-cloud::handoff::brief` input.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct HandoffBriefRequest {
    /// The hand-off's id, or the 8 characters of its marker line.
    pub handoff_id: String,
}

/// Why the hand-off's latest local run failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunFailure {
    pub stage: StageName,
    pub code: String,
    pub message: String,
    /// The run's last lines, oldest first, redacted.
    pub log_tail: Vec<String>,
}

/// `iii-cloud::handoff::brief` output: the brief, and the failed run when there is one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HandoffBriefResponse {
    #[serde(flatten)]
    pub brief: HandoffBrief,
    pub run_failure: Option<RunFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BriefProblem {
    pub id: String,
    pub code: String,
    pub owner: Owner,
    pub severity: Severity,
    pub title: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub detail: Option<String>,
    pub fix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BriefDeploy {
    pub id: String,
    pub stage: Option<String>,
    pub worker: Option<String>,
    pub code: Option<String>,
    pub title: Option<String>,
    pub hint: Option<String>,
    pub log_tail: Vec<String>,
    /// The release command from the manifest, when the failure is the release's.
    #[serde(default)]
    pub release_command: Option<String>,
    #[serde(default)]
    pub silent: bool,
}

/// A failed deploy in plain words (spec: readable failures).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeployFailure {
    pub code: String,
    pub stage: String,
    pub worker: Option<String>,
    /// Plain, per code: "The release command failed: `pnpm migrate`".
    pub title: String,
    pub hint: String,
    /// The last ~40 lines of the failing stage's log, oldest first, redacted.
    pub log_tail: Vec<String>,
    /// The log shows an exit status but no error text.
    #[serde(default)]
    pub silent: bool,
    /// The release command from the manifest, for a release failure.
    #[serde(default)]
    pub release_command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BriefBase {
    pub branch: String,
    pub sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BriefWorkspace {
    pub path: String,
    pub branch: String,
}

/// Everything the agent needs for a hand-off, so the prompt can be one plain
/// sentence plus the marker. No secret value is in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HandoffBrief {
    pub handoff_id: String,
    /// The base branch and its sha when the worktree was made: reset to it to
    /// start again (`OLD_ATTEMPT_COMMITS`).
    #[serde(default)]
    pub base: Option<BriefBase>,
    pub kind: HandoffKind,
    pub repository: Option<String>,
    /// `<project> · <environment>`.
    pub project: Option<String>,
    /// Where to work.
    pub worktree: Option<BriefWorkspace>,
    /// The person's own checkout: do not touch it.
    pub checkout_path: Option<String>,
    /// The compose file checked and deployed, relative to the worktree: the
    /// hand-off's, else the environment's.
    pub manifest: String,
    /// The environment is not production: it gets an assisted deploy, no pull
    /// request. Push only your fix branch; the person's deploy on the page sends
    /// the checked commit to the environment's own branch.
    #[serde(default)]
    pub pushes_directly: bool,
    /// Other compose files tracked in the worktree that declare `containers:`
    /// (at most 10). They are not checked or deployed.
    pub other_compose_files: Vec<String>,
    /// One line when an earlier try left the fix branch on origin: the first
    /// push replaces it with `--force-with-lease`.
    #[serde(default)]
    pub remote_branch_note: Option<String>,
    pub check_id: Option<String>,
    pub commit: Option<String>,
    /// The problems that are the agent's to fix.
    pub agent_problems: Vec<BriefProblem>,
    /// The ones the person decides: bring each with a recommendation.
    pub person_problems: Vec<BriefProblem>,
    pub deploy: Option<BriefDeploy>,
    pub next_step: String,
}

// ----------------------------------------------------------------- records

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConnStatus {
    #[default]
    SignedOut,
    Waiting,
    Connected,
    Expired,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Browser,
    ApiKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TeamRole {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApiKeyRef {
    pub secret: String,
    pub team_id: String,
    pub environment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Device {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Connection {
    pub status: ConnStatus,
    pub method: Option<Method>,
    pub cloud_url: String,
    pub user: Option<User>,
    pub teams: Vec<TeamRole>,
    pub api_key: Option<ApiKeyRef>,
    pub device: Option<Device>,
    pub expires_at: Option<String>,
    pub error: Option<WorkerError>,
}

impl Connection {
    pub fn signed_out(cloud_url: &str) -> Self {
        Self {
            status: ConnStatus::SignedOut,
            method: None,
            cloud_url: cloud_url.to_string(),
            user: None,
            teams: vec![],
            api_key: None,
            device: None,
            expires_at: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TeamRef {
    pub id: String,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CloudProject {
    pub id: String,
    pub name: String,
    /// The console's `$project` URL segment (`endpoint_slug`).
    #[serde(default)]
    pub slug: String,
    /// When the project was made (for choosing between several).
    #[serde(default)]
    pub created_at: Option<String>,
    /// Its newest deployment, when more than one project uses the repository.
    #[serde(default)]
    pub last_deployment_at: Option<String>,
}

/// A live cloud project of the person's teams that deploys another repository:
/// it can't be used with this folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OtherCloudProject {
    pub id: String,
    pub name: String,
    pub team: TeamName,
    pub repository: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TeamName {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnvironmentFull {
    pub id: String,
    pub name: String,
    pub branch: String,
    pub manifest: Option<String>,
    pub production: bool,
    /// Its branch's head on origin, read on refresh. Null before the first
    /// refresh, or when origin did not answer or has no such branch.
    #[serde(default)]
    pub head: Option<String>,
}

/// A cloud project of the repository, for the page's project and environment selectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CloudProjectInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub slug: String,
    pub team: TeamName,
    pub environments: Vec<EnvironmentFull>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub last_deployment_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnvironmentInfo {
    pub id: String,
    pub name: String,
    pub production: bool,
    pub branch: String,
    pub manifest: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnvironmentBrief {
    pub id: String,
    pub name: String,
    pub production: bool,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Checkout {
    pub branch: Option<String>,
    pub head: String,
    pub changed_files: Vec<String>,
    pub ahead: u32,
    /// The checkout's commits the selected environment does not serve
    /// (`<served>..HEAD`, newest first, at most 50), read on refresh. Null when
    /// nothing is served yet or the served commit could not be fetched.
    #[serde(default)]
    pub not_served: Option<Vec<CommitLine>>,
    /// Local branches and origin's, newest commit first, at most 30; a local
    /// branch hides its origin twin. Read on refresh, from the refs as they are.
    #[serde(default)]
    pub branches: Vec<BranchRef>,
}

/// A branch `project::use {from: checkout, source: name}` can take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BranchRef {
    /// `main`, or `origin/feature-x` for a branch only on origin.
    pub name: String,
    pub remote: bool,
    pub sha: String,
    pub committed_at: String,
    /// Commits the selected environment's branch (`origin/<branch>`) does not
    /// have; null when that branch is not on origin.
    pub ahead: Option<u32>,
    /// The selected environment deploys this branch.
    pub is_env_branch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ImportableManifest {
    pub path: String,
    pub choosable: bool,
    pub suggested_environment: Option<String>,
    pub issue: Option<String>,
    /// Its containers, as the file on this machine says (else as iii cloud
    /// lists them); null when neither knows.
    #[serde(default)]
    pub workers: Option<u32>,
    /// The file on this machine has a `cloud:` section.
    #[serde(default)]
    pub has_cloud_block: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Importable {
    pub team_id: String,
    pub repository_id: i64,
    pub manifests: Vec<ImportableManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Workspace {
    pub mode: WorkspaceMode,
    pub path: String,
    pub branch: String,
    /// The last commit the worktree took from the person's checkout
    /// (`from: checkout`): commits after it are the agent's. Null for a worktree made from the environment's branch.
    #[serde(default)]
    pub from_head: Option<String>,
    /// The branch `use {from: checkout, source}` took; null: the checkout's HEAD.
    #[serde(default)]
    pub source: Option<String>,
    /// The commit `source` (else the checkout's HEAD) pointed at when the
    /// worktree was prepared: "moved since" compares the source's tip with it.
    #[serde(default)]
    pub source_sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    pub path: String,
    /// `owner/name`.
    pub repository: String,
    pub checkout: Checkout,
    pub team: Option<TeamRef>,
    pub teams: Vec<TeamRef>,
    pub cloud_project: Option<CloudProject>,
    /// Every cloud project that uses this repository, across the person's
    /// teams, with its environments. The chosen one is `cloud_project`;
    /// `project::use {cloud_project_id}` picks another.
    #[serde(default)]
    pub cloud_projects: Vec<CloudProjectInfo>,
    /// Live projects of the person's teams that deploy another repository.
    #[serde(default)]
    pub other_cloud_projects: Vec<OtherCloudProject>,
    /// The repository's default branch.
    #[serde(default)]
    pub default_branch: Option<String>,
    pub environment: Option<EnvironmentInfo>,
    pub environments: Vec<EnvironmentBrief>,
    pub importable: Option<Importable>,
    pub github_access: Option<bool>,
    pub workspace: Option<Workspace>,
    /// Only in `project::use`'s answer: the `environment_id` asked for, which
    /// the cloud project no longer lists, so its default environment was taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fell_back_from: Option<String>,
}

/// A project the person used before, to switch back with `project::use {path,
/// cloud_project_id, environment_id, mode}` (no `resolve` needed first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecentProject {
    /// The repository's root on this machine.
    pub path: String,
    /// `owner/name`.
    pub repository: String,
    pub cloud_project_id: String,
    pub cloud_project_name: String,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub team_name: Option<String>,
    #[serde(default)]
    pub environment_id: Option<String>,
    #[serde(default)]
    pub environment_name: Option<String>,
    pub mode: WorkspaceMode,
    pub used_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Block,
    Warn,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    Agent,
    Person,
    /// Info only: nothing to do.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    CloudPlan,
    CheckHere,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Excerpt {
    pub start_line: u32,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Problem {
    /// sha1(source, code, file, line), stable across checks.
    pub id: String,
    pub source: Source,
    pub code: String,
    pub cloud_code: Option<String>,
    pub severity: Severity,
    pub owner: Owner,
    pub title: String,
    pub detail: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub excerpt: Option<Excerpt>,
    pub fix: Option<String>,
    /// The person skipped its name for now (`values::skip`).
    /// `EMPTY_DEFAULT_VARIABLE` then has severity `info`; `SECRET_MISSING` stays `block`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Clean,
    Blocked,
    NotPushed,
    CommitNotPlanned,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ResourceConfirmation {
    pub kind: String,
    pub alias: String,
    pub binding: Option<String>,
    pub tier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OverrideConfirmation {
    pub setting: String,
    pub console: Value,
    pub file: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Confirmations {
    pub resources: Vec<ResourceConfirmation>,
    pub overrides: Vec<OverrideConfirmation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlanPart {
    pub status: PlanStatus,
    pub fingerprint: Option<String>,
    pub digest: Option<String>,
    pub blocking: u32,
    pub warnings: u32,
    /// The plan stops at its first blocking step: more may follow.
    pub at_least: bool,
    pub confirmations: Confirmations,
    pub web_candidates: Option<Value>,
    pub error: Option<WorkerError>,
    /// What the plan runs: its workers and machine groups. Null without a plan.
    #[serde(default)]
    pub summary: Option<PlanSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanSummary {
    pub workers: u32,
    /// The groups' names, in the plan's order.
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HerePart {
    pub problems: u32,
    pub iii_version: Option<String>,
    pub accepts_cloud_block: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StageName {
    Install,
    Build,
    Release,
    ComposeUp,
    Workers,
    Routes,
    Healthcheck,
}

impl StageName {
    pub const ALL: [StageName; 7] = [
        Self::Install,
        Self::Build,
        Self::Release,
        Self::ComposeUp,
        Self::Workers,
        Self::Routes,
        Self::Healthcheck,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Pending,
    Running,
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Stage {
    pub name: StageName,
    pub status: StageStatus,
    pub detail: Option<String>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkerCount {
    pub ready: u32,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RouteCount {
    pub count: u32,
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HealthResult {
    pub path: String,
    pub status: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunError {
    pub stage: StageName,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Run {
    pub id: String,
    pub check_id: String,
    pub commit: String,
    pub engine: String,
    pub status: RunStatus,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub stages: Vec<Stage>,
    pub workers: WorkerCount,
    pub routes: RouteCount,
    pub healthcheck: Option<HealthResult>,
    pub not_covered: Vec<String>,
    pub log_tail: Vec<String>,
    pub error: Option<RunError>,
    /// A passed run of an earlier check answered this one: nothing ran again.
    #[serde(default)]
    pub reused: bool,
    /// The workers stage has gone a minute with no new worker: who it waits for.
    /// The run is still `running`; null otherwise.
    #[serde(default)]
    pub waiting_on: Option<WaitingOn>,
    /// The tracked files the run rewrote and the worker put back as committed when it ended
    /// (at most 500; their rewritten versions are kept in the run's `left/` folder).
    #[serde(default)]
    pub restored: Vec<String>,
    /// The untracked files the run created, moved out of the copy into the run's `left/` folder
    /// when it ended (at most 500).
    #[serde(default)]
    pub moved: Vec<String>,
    /// The run's whole redacted log, an absolute path (`log_tail` is only its last 40 lines).
    #[serde(default)]
    pub log_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WaitingOn {
    /// The containers that have not connected.
    pub containers: Vec<String>,
    /// When the wait was noticed (RFC 3339).
    pub since: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Check {
    pub id: String,
    pub environment_id: String,
    pub commit: String,
    /// The environment branch's head when checked.
    pub base: String,
    pub checked_at: String,
    pub duration_ms: u64,
    pub plan: PlanPart,
    pub here: HerePart,
    /// Cloud plan first, then the checks here.
    pub problems: Vec<Problem>,
    pub run: Option<Run>,
    pub ready: bool,
    pub report: String,
    /// The compose file it was checked with, from the repository root. When it
    /// is not the environment's, deploying this check switches the environment to it.
    #[serde(default)]
    pub manifest: Option<String>,
    /// The hand-off whose agent asked for it; null for the page's own checks.
    #[serde(default)]
    pub handoff_id: Option<String>,
    /// The commits of `base..commit`, newest first, at most 50. Empty when the
    /// base was not read or the commit is not on this machine.
    #[serde(default)]
    pub commits: Vec<CommitLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CommitLine {
    pub sha: String,
    /// The subject line.
    pub message: String,
}

/// A check in short, for the page's history (`Status.checks`); the whole
/// record is `iii-cloud::check::get {id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CheckSummary {
    pub id: String,
    pub environment_id: String,
    pub commit: String,
    pub checked_at: String,
    /// The plan and the checks here; a run's time is in its stages.
    pub duration_ms: u64,
    pub ready: bool,
    /// The problems someone acts on (owner `agent` or `person`).
    pub problems: u32,
    pub handoff_id: Option<String>,
    pub manifest: Option<String>,
}

impl From<&Check> for CheckSummary {
    fn from(c: &Check) -> Self {
        Self {
            id: c.id.clone(),
            environment_id: c.environment_id.clone(),
            commit: c.commit.clone(),
            checked_at: c.checked_at.clone(),
            duration_ms: c.duration_ms,
            ready: c.ready,
            problems: c.problems.iter().filter(|p| p.owner != Owner::None).count() as u32,
            handoff_id: c.handoff_id.clone(),
            manifest: c.manifest.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecretState {
    Waiting,
    Set,
    LeftEmpty,
    Noted,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CloudOutcome {
    Set,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LocalOutcome {
    Set,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretRequest {
    pub id: String,
    pub environment_id: String,
    pub name: String,
    pub description: String,
    pub mode: SecretMode,
    pub state: SecretState,
    pub cloud: Option<CloudOutcome>,
    pub local: Option<LocalOutcome>,
    pub asked_at: String,
    pub resolved_at: Option<String>,
    pub error: Option<WorkerError>,
    /// Asked for a database URL the person chose to keep outside iii cloud.
    #[serde(default)]
    pub external: bool,
    /// Where the value goes: the team, worktree and environment the request was
    /// asked for, whatever the page has selected when the value is saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<crate::check::Selected>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrChecks {
    None,
    Pending,
    Passing,
    Failing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    pub title: String,
    pub state: PrState,
    pub head: String,
    pub head_sha: String,
    pub base: String,
    pub body: String,
    pub checks: PrChecks,
    pub mergeable: Option<String>,
    pub main_moved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VariableRow {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String,
    pub required_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeployCommit {
    pub sha: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeployStage {
    pub name: String,
    pub status: String,
    pub detail: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseRowStatus {
    Pending,
    Running,
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReleaseRow {
    pub status: ReleaseRowStatus,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Running,
    Ready,
    Failed,
    NoRoute,
    Canceled,
    Replaced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeployError {
    pub stage: String,
    pub worker: Option<String>,
    pub code: String,
    pub hint: String,
    pub action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Deploy {
    pub id: String,
    pub number: u64,
    pub environment_id: String,
    pub commit: DeployCommit,
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String,
    pub status_detail: Option<String>,
    pub stages: Vec<DeployStage>,
    pub release_row: Option<ReleaseRow>,
    pub outcome: Outcome,
    pub error: Option<DeployError>,
    pub log_tail: Vec<String>,
    /// For a failed deploy: what failed, in plain words, with the failing stage's log.
    #[serde(default)]
    pub failure: Option<DeployFailure>,
    pub pr: Option<u64>,
    pub approved_by_worker: bool,
    pub checked_ready: bool,
    pub created_at: String,
    pub finished_at: Option<String>,
    /// The deployment that replaced this one, when it is `replaced`.
    #[serde(default)]
    pub replaced_by: Option<String>,
    pub console_url: String,
    /// This worker did not create it: made in the console (the cloud's `type` is
    /// `manual` for both).
    #[serde(default)]
    pub from_console: bool,
    /// The check this worker deployed (`deploy {check_id}`); null for a
    /// deployment made elsewhere.
    #[serde(default)]
    pub check_id: Option<String>,
}

impl Deploy {
    /// Still moving: running, or waiting for an action (`needs_action`, the
    /// page's `needsAction`), which is a running outcome too.
    pub fn is_moving(&self) -> bool {
        self.outcome == Outcome::Running || self.status == "needs_action"
    }
}

/// Where a deploy was made from: kept by deploy id, so it is still found and
/// followed after the person switches to another project or environment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployOrigin {
    pub team_id: String,
    pub team_slug: Option<String>,
    pub cloud_project_id: Option<String>,
    pub project_slug: Option<String>,
    pub project_name: Option<String>,
    pub repository: String,
    pub environment_name: String,
    pub path: String,
}

/// A deploy still moving, with where it was made from. A field is null when
/// the worker never knew it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunningDeploy {
    pub deploy: Deploy,
    pub cloud_project_id: Option<String>,
    pub project_name: Option<String>,
    pub repository: Option<String>,
    pub environment_name: Option<String>,
    /// The local folder it was made from: `project::use {path}` goes back to it.
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkerInfo {
    pub version: String,
    pub iii_version: Option<String>,
    pub cloud_engine: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Status {
    pub worker: WorkerInfo,
    /// The worker's `auto_fix` setting: a person's check hands the agent its problems.
    pub auto_fix: bool,
    pub connection: Connection,
    pub project: Option<Project>,
    /// The selected environment's latest check.
    pub check: Option<Check>,
    /// Every kept check in short, of every environment, newest first (the
    /// latest 20, and the ones a merge, hand-off, pull request or deploy points at).
    #[serde(default)]
    pub checks: Vec<CheckSummary>,
    /// Projects used before, newest first: not the current one, nor a folder
    /// that is gone.
    #[serde(default)]
    pub recent_projects: Vec<RecentProject>,
    /// A folder `project::resolve` read while another project is selected:
    /// Setup's add-project steps show it. `project::use` selects it and clears this.
    #[serde(default)]
    pub candidate: Option<Project>,
    /// Open first, then the latest 20 resolved.
    pub secrets: Vec<SecretRequest>,
    pub variables: Vec<VariableRow>,
    pub pr: Option<PullRequest>,
    pub pr_error: Option<WorkerError>,
    /// What is wrong with the selection itself: `PROJECT_DELETED`, `ENVIRONMENT_DELETED`.
    #[serde(default)]
    pub problems: Vec<WorkerError>,
    /// Newest first, at most 20.
    pub deploys: Vec<Deploy>,
    /// The agent's work on the selected environment's open hand-off.
    pub agent: Option<AgentState>,
    /// What the agent needs for the open hand-off, in one place.
    pub handoff_brief: Option<HandoffBrief>,
    pub refreshed_at: Option<String>,
    /// The worktree's tracked compose files that declare `containers:`, for the
    /// page's app file picker. `choosable` is false for a file iii cloud can't
    /// plan by its name (`issue`: `NOT_WORKER_COMPOSE`).
    #[serde(default)]
    pub compose_files: Vec<ImportableManifest>,
    /// The names the person skipped for now (`values::skip`) in the selected environment, sorted.
    #[serde(default)]
    pub skipped_values: Vec<String>,
    /// Every deploy still moving, of every project and environment, newest
    /// first (`deploys` has the selected environment's only).
    #[serde(default)]
    pub running_deploys: Vec<RunningDeploy>,
    /// The person's choice per console override of the selected environment
    /// (`override::choose`), by setting: only choices made for the newest
    /// check's console and file values.
    #[serde(default)]
    pub override_choices: BTreeMap<String, OverrideChoice>,
    /// The warning codes the person skipped for now (`problem::skip`) in the
    /// selected environment, sorted.
    #[serde(default)]
    pub skipped_problems: Vec<String>,
    /// The compose file the selected environment's work is on when it is not
    /// the environment's own (`State::work_file`): the newest check's, unless
    /// the environment's file changed since. Null: the environment's file.
    #[serde(default)]
    pub work_file: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Agent,
    Connection,
    Project,
    Check,
    Run,
    Secret,
    Deploy,
}

/// `iii-cloud::on-change` event: one record changed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChangeEvent {
    pub kind: EventKind,
    /// The record's id (`current` for `connection` and `project`).
    pub id: String,
    /// A run or deploy finished, a secret resolved, the connection settled.
    pub terminal: bool,
    /// One line, e.g. "deploy 7a1f3b2 failed at workers: OOMKilled".
    pub summary: String,
    /// Connection, Project, Check, Run, SecretRequest or Deploy.
    pub record: Value,
}

// ------------------------------------------------------------------ values

/// Where `iii-cloud::values` reads the compose file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValuesFrom {
    /// The selected workspace (the worktree, or the checkout in "edits this folder").
    #[default]
    Workspace,
    /// The person's checkout.
    Checkout,
}

/// `iii-cloud::values` input.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ValuesRequest {
    /// A `worker-compose*.yaml` from the repository root. Default: the newest
    /// check's file when it is of this environment, else the environment's.
    #[serde(default)]
    pub manifest: Option<String>,
    /// Default `workspace`.
    #[serde(default)]
    pub from: Option<ValuesFrom>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CloudRead {
    Read,
    /// iii cloud's variables could not be read: every `in_cloud` is `unknown`.
    Unreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SaveAllowed {
    Ok,
    /// An API key can't write values.
    ApiKey,
    /// Production's values take Owner or Admin.
    Role,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    /// `${X}`
    Required,
    /// `${X:-}`
    Optional,
    /// `${X:-v}`
    HasDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    Secret,
    Variable,
}

impl ValueKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ValueKind::Secret => "secret",
            ValueKind::Variable => "variable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InCloud {
    Set,
    Unset,
    Unknown,
}

/// An open `iii-cloud::secret` ask for the name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValueAsked {
    pub request_id: String,
    pub description: String,
}

/// Where a value can come from. Never the value itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum Suggestion {
    /// A `.env*` file of the person's folder has the name.
    EnvFile {
        /// From the repository root, such as `api/.env.test`.
        file: String,
        preselect: bool,
        /// The value points at this machine (`localhost`, `127.0.0.1`, ...).
        local: bool,
    },
    /// The environment's own address.
    EnvUrl { url: String, preselect: bool },
    /// Another environment of the project has it set (copied by iii cloud).
    Copy {
        environment_id: String,
        environment: String,
        preselect: bool,
    },
}

/// One name the compose file reads (or an agent asked for).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValueItem {
    pub name: String,
    pub need: Need,
    /// The file's literal default; `""` for `${X:-}`; null for `${X}`.
    pub default: Option<String>,
    /// Secret unless iii cloud stores it as a variable or it is the environment's address.
    #[serde(rename = "type")]
    pub kind: ValueKind,
    /// Shaped like a database URL: iii cloud makes the database.
    pub database: bool,
    pub containers: Vec<String>,
    pub line: Option<u32>,
    pub in_cloud: InCloud,
    /// Only a set variable's value in iii cloud; never a secret's.
    pub value: Option<String>,
    pub updated_at: Option<String>,
    pub asked: Option<ValueAsked>,
    /// In preselect order: the page takes the first with `preselect: true`.
    pub suggestions: Vec<Suggestion>,
    /// Why iii cloud can't store this name (lower case, say): nothing is
    /// preselected and `values::save` refuses the row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid_name: Option<String>,
    /// Whether the run here has a value for it.
    pub here: Here,
    /// With `here: from_file`: the person's env file it comes from, such as `api/.env.test`.
    pub here_file: Option<String>,
    /// The person skipped it for now (`values::skip`).
    pub skipped: bool,
}

/// Whether the run here has a value for a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Here {
    /// In the run's `.env`.
    Set,
    /// Taken from an env file of the person's folder, into the run's environment only.
    FromFile,
    /// Nowhere here: `LOCAL_VARIABLE_UNSET`.
    Unset,
    /// It has a default (or the compose file doesn't read it).
    NotNeeded,
}

/// `iii-cloud::values` answer: names, sources and flags, never a secret's value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Values {
    pub environment_id: String,
    pub environment: String,
    pub manifest: String,
    pub read_from: ValuesFrom,
    pub cloud: CloudRead,
    pub save: SaveAllowed,
    pub items: Vec<ValueItem>,
}

/// One value `iii-cloud::values::save` writes. No `Debug`: it can hold a value.
#[derive(Clone, Deserialize, JsonSchema)]
#[serde(tag = "from", rename_all = "snake_case")]
pub enum SaveItem {
    Typed {
        name: String,
        #[serde(rename = "type")]
        kind: ValueKind,
        value: String,
    },
    /// Read again from the file at save time.
    EnvFile {
        name: String,
        #[serde(rename = "type")]
        kind: ValueKind,
        file: String,
    },
    /// The environment's address, as a variable.
    EnvUrl { name: String },
    /// The variable `""`.
    LeaveEmpty { name: String },
    /// Copied by iii cloud from another environment of the project; never overwrites.
    Copy {
        name: String,
        environment_id: String,
    },
}

impl SaveItem {
    pub fn name(&self) -> &str {
        match self {
            SaveItem::Typed { name, .. }
            | SaveItem::EnvFile { name, .. }
            | SaveItem::EnvUrl { name }
            | SaveItem::LeaveEmpty { name }
            | SaveItem::Copy { name, .. } => name,
        }
    }
}

/// `iii-cloud::values::save` input: the page's own. No `Debug`.
#[derive(Deserialize, JsonSchema)]
pub struct ValuesSave {
    /// The selected environment, as a guard (`ENVIRONMENT_CHANGED`).
    pub environment_id: String,
    #[serde(default)]
    pub manifest: Option<String>,
    pub items: Vec<SaveItem>,
}

/// `iii-cloud::values::skip` input: the page's own. Names only.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ValuesSkip {
    /// The selected environment, as a guard (`ENVIRONMENT_CHANGED`).
    pub environment_id: String,
    pub names: Vec<String>,
    /// `false` undoes the skip.
    pub skip: bool,
}

/// Which value of a console override the person keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OverrideChoice {
    /// The file's value wins at deploy (what the deploy does already).
    File,
    /// The console's value is kept, which takes a change to the file.
    Console,
}

/// `iii-cloud::override::choose` input: the page's own.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OverrideChoose {
    /// The selected environment, as a guard (`ENVIRONMENT_CHANGED`).
    pub environment_id: String,
    /// The override's setting (`web`), as the newest check lists it.
    pub setting: String,
    pub choice: OverrideChoice,
}

/// The environment's choices after `iii-cloud::override::choose`, as
/// `Status.override_choices`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OverrideChosen {
    pub environment_id: String,
    pub override_choices: BTreeMap<String, OverrideChoice>,
}

/// `iii-cloud::config::import` input: the page's own.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigImport {
    /// The selected environment, as a guard (`ENVIRONMENT_CHANGED`).
    pub environment_id: String,
    /// Preview only: nothing is written.
    pub dry_run: bool,
    /// The `version` the dry run returned. Needed to apply.
    #[serde(default)]
    pub expected_version: Option<i64>,
}

/// What `iii-cloud::config::import` copies. Values only in a dry run's
/// preview, never a secret-looking one: no `Debug`, so they never reach a log.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigImported {
    pub environment_id: String,
    /// The config folder, from the repository root (`api/config`).
    pub directory: String,
    /// The config version the dry run read, or the one the apply made.
    pub version: i64,
    /// False for a dry run.
    pub applied: bool,
    /// iii cloud has unapplied console edits for this environment: applying
    /// is refused until they're applied or dropped there.
    pub draft_changed: bool,
    pub workers: Vec<ConfigImportWorker>,
    /// Files of the folder not copied, and why.
    pub skipped: Vec<ConfigSkippedFile>,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigImportWorker {
    /// The container key iii cloud keeps the config by.
    pub worker: String,
    /// The file, from the repository root.
    pub file: String,
    /// The fields copied, as dotted keys; lists stay JSON values. `value` is
    /// null once applied.
    pub fields: Vec<ConfigImportField>,
    /// Secret-looking keys or credential-looking values, by key only.
    pub left_out: Vec<String>,
    /// Keys iii cloud or the compose file set, which the console can't change.
    pub set_elsewhere: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigImportField {
    pub key: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigSkippedFile {
    pub file: String,
    pub reason: ConfigSkipReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSkipReason {
    /// No container iii cloud runs has that config entry.
    NotAWorker,
    /// A link, or outside the repository.
    NotAPlainFile,
    /// Over 1 MiB.
    TooLarge,
    /// Not a configuration entry (`id`, `name`, `description`, `value`).
    Unreadable,
}

/// `iii-cloud::problem::skip` input: the page's own. Only the warnings in
/// `values::SKIPPABLE_PROBLEMS` (`LOCAL_CONFIG_NOT_SET`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProblemSkip {
    /// The selected environment, as a guard (`ENVIRONMENT_CHANGED`).
    pub environment_id: String,
    pub code: String,
    /// `false` undoes the skip.
    pub skip: bool,
}

/// The environment's skipped warning codes after `iii-cloud::problem::skip`, sorted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProblemsSkipped {
    pub environment_id: String,
    pub codes: Vec<String>,
}

/// The environment's skipped names after `iii-cloud::values::skip`, sorted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValuesSkipped {
    pub environment_id: String,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SaveResult {
    pub name: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<WorkerError>,
}

/// `iii-cloud::values::save` answer, per item in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValuesSaved {
    pub environment_id: String,
    pub results: Vec<SaveResult>,
}
