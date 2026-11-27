// Copied from tech-specs/2026-11-iii-cloud-worker/spec.md §11; the golden
// schemas in iii-cloud/tests/golden/schemas are the source of truth.

export type WorkerError = {
  code: string
  message: string
  status?: number
  fix?: string
  /** `RUN_BUSY`: the run that is already going, and when it started. */
  run_id?: string
  started_at?: string
}

export type Status = {
  worker: { version: string; iii_version: string | null; cloud_engine: string | null }
  connection: Connection
  project: Project | null
  check: Check | null // the selected environment's latest
  /** Every kept check in short, of every environment, newest first; the whole record is `iii-cloud::check::get {id}`. */
  checks?: CheckSummary[]
  /** What is wrong with the selection itself: `PROJECT_DELETED`, `ENVIRONMENT_DELETED`. */
  problems?: WorkerError[]
  secrets: SecretRequest[] // open first, then the latest 20 resolved
  variables: {
    name: string
    type: 'secret' | 'variable' | 'builtin'
    status: 'set' | 'missing' | 'empty_default'
    required_by: string[]
  }[]
  pr: PullRequest | null
  pr_error: WorkerError | null
  deploys: Deploy[] // newest first, at most 20
  refreshed_at: string | null
  agent?: AgentRun | null
  handoff_brief?: HandoffBrief | null
  /** The worker's `auto_fix` setting; absent on an older worker (read as on). */
  auto_fix?: boolean
  /** The worktree's compose files for the app file picker; `choosable` false says why in `issue`. */
  compose_files?: ComposeFile[]
  /** Folders and cloud projects used before, newest first: never the current one, nor a folder that is gone. */
  recent_projects?: RecentProject[]
  /** A folder `project::resolve` read while another project is selected: Setup's add-project steps show it. `project::use` selects it and clears this. */
  candidate?: Project | null
  /** The names the person skipped for now (`values::skip`) in the selected environment, sorted. */
  skipped_values?: string[]
  /** The compose file the environment's work is on when it is not the environment's own; null: the environment's. */
  work_file?: string | null
  /** Every deploy still moving (`outcome: 'running'`, which includes `needs_action`), of every project and environment, newest first. `deploys` has only the selected environment's. */
  running_deploys?: RunningDeploy[]
  /** The person's choice per console override of the selected environment (`override::choose`), by setting. Only
      choices made for the newest check's console and file values: a changed pair drops out. */
  override_choices?: Record<string, OverrideChoice>
  /** The cloud warning codes the person skipped for now (`problem::skip`) in the selected environment, sorted. */
  skipped_problems?: string[]
}

/** A deploy still moving, with where it was made from. A field is null when the worker never knew it (a deploy it first saw in another session). */
export type RunningDeploy = {
  deploy: Deploy
  cloud_project_id: string | null
  project_name: string | null
  repository: string | null
  environment_name: string | null
  /** The local folder it was made from: `project::use {path, environment_id: deploy.environment_id}` goes back to it. */
  path: string | null
}

/** One `recent_projects` entry: `iii-cloud::project::use {path, cloud_project_id, environment_id, mode}` takes it up again, no `resolve` first. */
export type RecentProject = {
  path: string
  repository: string
  cloud_project_id: string
  cloud_project_name: string
  team_id?: string | null
  team_name?: string | null
  environment_id?: string | null
  environment_name?: string | null
  mode?: 'worktree' | 'checkout'
  used_at?: string
}

export type ComposeFile = {
  path: string
  choosable: boolean
  suggested_environment: string | null
  issue: string | null
  /** Its containers, as the file on this machine says (else as iii cloud lists them); null when neither knows. */
  workers?: number | null
  /** The file on this machine has a `cloud:` section. */
  has_cloud_block?: boolean
}

// iii cloud's answers the page reads (cloud-v1.yaml), only the fields it uses.
export type CloudDeployment = {
  id: string
  number: number
  environment_id: string
  type: string
  status: 'queued' | 'in_progress' | 'needs_action' | 'ready' | 'failed' | 'replaced' | 'canceled'
  /** The one that serves the environment. */
  current: boolean
  commit: { sha: string; branch?: string; message?: string }
  triggered_by: { user_id: string | null; api_key_id: string | null }
  created_at: string
  finished_at: string | null
}
export type DeploymentList = { deployments: CloudDeployment[]; next_cursor?: string | null }
export type Members = { members: { user: { id: string; email: string; name?: string; github_login?: string } }[] }

export type Connection = {
  status: 'signed_out' | 'waiting' | 'connected' | 'expired' | 'error'
  method: 'browser' | 'api_key' | null
  cloud_url: string
  user: { id: string; email: string; name: string | null } | null
  teams: { id: string; name: string; slug: string; role: string }[]
  api_key: { secret: string; team_id: string; environment_id: string } | null
  device: { user_code: string; verification_uri: string; expires_at: string } | null
  expires_at: string | null
  error: WorkerError | null
}

export type Project = {
  path: string
  repository: string // owner/name
  checkout: {
    branch: string | null
    head: string
    changed_files: string[]
    ahead: number
    /** The checkout's commits the selected environment does not serve, newest first; null when unknown. */
    not_served?: CommitLine[] | null
    /** Local branches and origin's, newest commit first, at most 30 (a local one hides its origin twin); read on refresh. */
    branches?: BranchRef[]
  }
  team: { id: string; name: string; slug: string } | null
  teams: { id: string; name: string; slug: string }[] // when team is null
  cloud_project: { id: string; name: string; slug: string } | null
  /** The repository's default branch: an assisted environment on it moves to its own branch on its first deploy. */
  default_branch?: string | null
  /** Every cloud project of this repository (more than one: the person picks). */
  cloud_projects?: CloudProjectChoice[]
  /** Projects of the person's other repositories: shown, never usable with this folder. */
  other_cloud_projects?: { id: string; name: string; repository: string }[]
  environment: {
    id: string
    name: string
    production: boolean
    branch: string
    manifest: string | null
    url: string | null
  } | null
  environments: { id: string; name: string; production: boolean; branch: string; manifest?: string | null }[]
  importable: {
    team_id: string
    repository_id: number
    manifests: ComposeFile[]
  } | null
  github_access: boolean | null
  workspace: {
    mode: 'worktree' | 'checkout'
    path: string
    branch: string
    /** `project::use {from: 'checkout'}`: the last commit taken from the checkout; later ones are the agent's. */
    from_head?: string | null
    /** The branch `use {from: 'checkout', source}` took instead of the checkout's HEAD; null: the checkout's HEAD. */
    source?: string | null
    /** The commit `source` (else the checkout's HEAD) pointed at when the worktree was prepared. "Moved since" compares it with the source's tip now: `checkout.branches[].sha` of `source`, or `checkout.head`. */
    source_sha?: string | null
  } | null
  /** Only in `project::use`'s answer: the `environment_id` asked for, no longer listed, so the default environment was taken. */
  fell_back_from?: string | null
}

/** A branch `use {from: 'checkout', source: name}` can take. */
export type BranchRef = {
  /** `main`, or `origin/feature-x` for one only on origin. */
  name: string
  remote: boolean
  sha: string
  committed_at: string
  /** Commits the selected environment's branch doesn't have; null when its branch isn't on origin. */
  ahead: number | null
  /** The selected environment deploys this branch. */
  is_env_branch: boolean
}

export type Problem = {
  id: string // sha1(source, code, file, line), stable across checks
  source: 'cloud_plan' | 'check_here'
  code: string
  cloud_code: string | null
  severity: 'block' | 'warn' | 'info'
  owner: 'agent' | 'person' | 'none'
  title: string
  detail: string | null
  file: string | null
  line: number | null
  excerpt: { start_line: number; lines: string[] } | null
  fix: string | null
  /** The person skipped its name for now (`values::skip`). `EMPTY_DEFAULT_VARIABLE` then has severity `info`; `SECRET_MISSING` stays `block` (iii cloud needs a value, even an empty one). */
  skipped?: boolean
}

/** `iii-cloud::check`'s request. `from_page`: the person asked for it on this page (the worker wakes a waiting agent only for such a check). */
export type CheckRequest = { run?: boolean; push?: boolean; manifest?: string | null; from_page?: boolean }

export type Check = {
  id: string
  environment_id: string
  commit: string
  base: string // env branch head when checked
  checked_at: string
  duration_ms: number
  plan: {
    status: 'clean' | 'blocked' | 'not_pushed' | 'commit_not_planned' | 'unavailable'
    fingerprint: string | null
    digest: string | null
    blocking: number
    warnings: number
    at_least: boolean
    confirmations: {
      resources: { kind: string; alias: string; binding: string | null; tier: string | null }[]
      overrides: { setting: string; console: unknown; file: unknown }[]
    }
    web_candidates: unknown | null
    error: WorkerError | null
    /** What the plan runs; null without a plan. */
    summary?: PlanSummary | null
  }
  here: { problems: number; iii_version: string | null; accepts_cloud_block: boolean }
  problems: Problem[] // cloud plan first, then the checks here
  run: Run | null
  ready: boolean
  report: string
  /** The compose file it was checked with; deploying it switches the environment when it is another one. */
  manifest?: string | null
  /** The hand-off whose agent asked for it; null for the page's own checks. */
  handoff_id?: string | null
  /** `base..commit`, newest first, at most 50; empty when the base was not read or the commit is not here. */
  commits?: CommitLine[]
}

export type PlanSummary = { workers: number; groups: string[] }

export type CommitLine = { sha: string; message: string }

/** A check in short (`Status.checks`). */
export type CheckSummary = {
  id: string
  environment_id: string
  commit: string
  checked_at: string
  /** The plan and the checks here; a run's time is in its stages. */
  duration_ms: number
  ready: boolean
  /** Problems someone acts on (owner `agent` or `person`). */
  problems: number
  handoff_id: string | null
  manifest: string | null
}

export type Stage = {
  name: 'install' | 'build' | 'release' | 'compose_up' | 'workers' | 'routes' | 'healthcheck'
  status: 'pending' | 'running' | 'passed' | 'failed' | 'skipped'
  detail: string | null
  duration_ms: number | null
}

export type Run = {
  id: string
  check_id: string
  commit: string
  engine: string
  status: 'running' | 'passed' | 'failed'
  started_at: string
  finished_at: string | null
  stages: Stage[]
  workers: { ready: number; total: number }
  routes: { count: number; prefix: string | null }
  healthcheck: { path: string; status: number | null } | null
  not_covered: string[]
  /** The commit's run had already passed and this answer is that run. A worker that does not send it: false. */
  reused?: boolean
  log_tail: string[]
  error: { stage: Stage['name']; code: string; message: string } | null
}

export type SecretRequest = {
  id: string
  environment_id: string
  name: string
  description: string
  mode: 'ask' | 'not_needed' | 'decide'
  state: 'waiting' | 'set' | 'left_empty' | 'noted' | 'failed'
  cloud: 'set' | 'skipped' | 'failed' | null
  local: 'set' | 'skipped' | null
  asked_at: string
  resolved_at: string | null
  error: WorkerError | null
  /** Asked for a database URL the person chose to keep outside iii cloud. */
  external: boolean
}

export type PullRequest = {
  number: number
  url: string
  title: string
  state: 'OPEN' | 'MERGED' | 'CLOSED'
  head: string
  head_sha: string
  base: string
  body: string
  checks: 'none' | 'pending' | 'passing' | 'failing'
  mergeable: string | null
  main_moved: boolean
}

export type Deploy = {
  id: string
  number: number
  environment_id: string
  commit: { sha: string; message: string | null }
  type: string
  status: 'queued' | 'in_progress' | 'needs_action' | 'ready' | 'failed' | 'replaced' | 'canceled' | 'gone'
  status_detail: string | null
  stages: {
    name: string
    status: string
    detail: string | null
    started_at: string | null
    finished_at: string | null
  }[]
  release_row: { status: 'pending' | 'running' | 'passed' | 'failed'; duration_ms: number | null } | null
  outcome: 'running' | 'ready' | 'failed' | 'no_route' | 'canceled' | 'replaced'
  error: { stage: string; worker: string | null; code: string; hint: string; action: string | null } | null
  log_tail: string[]
  pr: number | null
  replaced_by: string | null
  approved_by_worker: boolean
  checked_ready: boolean
  created_at: string
  finished_at: string | null
  console_url: string
  /** Started in the iii cloud console, not by this worker. */
  from_console?: boolean
  /** Why it failed, in words, with the log lines of the failing stage. */
  failure?: DeployFailure | null
  /** The check this worker deployed; null for a deployment made elsewhere. */
  check_id?: string | null
}

export type DeployFailure = {
  code: string
  stage: string
  worker: string | null
  title: string
  hint: string | null
  log_tail: string[]
  /** The command exited without printing an error. */
  silent?: boolean
  /** The release command from the manifest, for a release failure. */
  release_command?: string | null
}

export type ChangeKind = 'connection' | 'project' | 'check' | 'run' | 'secret' | 'deploy' | 'agent'

export type ChangeEvent = {
  kind: ChangeKind
  id: string
  terminal: boolean
  summary: string
  record: Connection | Project | Check | Run | SecretRequest | Deploy | AgentRun
}

// The agent's live hand-off (`status.agent`), from `iii-cloud::handoff`.
export type AgentState = 'starting' | 'working' | 'waiting_for_you' | 'waiting' | 'stalled' | 'done' | 'stopped'

export type AgentRun = {
  handoff_id: string
  session_id: string | null
  state: AgentState
  started_at: string
  last_activity_at: string | null
  steps: number
  current: string | null
  question: string | null
  /** Problem ids the agent is on right now (empty: unknown, never guess). */
  working_on?: string[]
  /** What the hand-off was for: where the row belongs on the page. */
  kind?: HandoffKind
  /** A short plain line about what it is doing. */
  note?: string | null
  /** What it waits for when `state` is `waiting` (a wake it armed): "the run here to end". */
  waiting_for?: string | null
}

export type HandoffKind = 'ready' | 'deploy_fix' | 'pull_request' | 'import'
export type HandoffResult = {
  id: string
  marker: string
  /** The worker put the text into the agent's own chat: no new chat is needed. */
  delivered?: boolean
  /** The worker already had this hand-off open: it delivered the text itself; never start another chat. */
  reused?: boolean
  session_id?: string | null
}

export type BriefProblem = {
  /** The problem's code and owner, as the worker sends them (absent from an older brief). */
  code?: string | null
  owner?: 'agent' | 'person' | 'none'
  title: string
  file: string | null
  line: number | null
  detail: string | null
  fix: string | null
}

/** Everything the agent needs for a hand-off (`status.handoff_brief`); the chat text stays one sentence. */
export type HandoffBrief = {
  handoff_id: string
  kind: HandoffKind
  repository: string | null
  project: string | null
  worktree: { path: string; branch: string } | null
  checkout_path: string | null
  check_id: string | null
  commit: string | null
  agent_problems: BriefProblem[]
  person_problems: BriefProblem[]
  deploy: {
    id: string
    stage: string | null
    worker: string | null
    code: string | null
    hint: string | null
    log_tail: string[]
  } | null
  next_step: string
  /** Not production: an assisted deploy, no pull request; the person's deploy sends the checked commit to the environment's own branch. (The page decides the way from `production`, not from this.) */
  pushes_directly?: boolean
}

/** A cloud project that uses this repository, with its environments (the person may pick any). */
export type CloudProjectChoice = {
  id: string
  name: string
  slug: string
  team?: { id: string; name: string } | null
  environments?: {
    id: string
    name: string
    production?: boolean
    branch: string
    manifest?: string | null
    deleted?: boolean
    /** Its branch's head on origin, read on refresh; null when unknown. */
    head?: string | null
  }[]
  created_at?: string | null
  last_deployment_at?: string | null
  /** Core drops deleted ones; the page guards anyway. */
  deleted?: boolean
}

/** `iii-cloud::values {manifest?, from?}`: the names the compose file reads, and where each value can come from. Never a secret's value. */
export type Values = {
  environment_id: string
  environment: string
  manifest: string
  read_from: 'workspace' | 'checkout'
  /** `unreadable`: iii cloud's variables could not be read; every `in_cloud` is `unknown`. */
  cloud: 'read' | 'unreadable'
  /** `api_key`: an API key can't write values. `role`: production's values take Owner or Admin. */
  save: 'ok' | 'api_key' | 'role'
  items: ValueItem[]
}

export type ValueItem = {
  name: string
  /** `${X}` | `${X:-}` | `${X:-v}` */
  need: 'required' | 'optional' | 'has_default'
  /** The file's literal default; "" for `${X:-}`; null for `${X}`. */
  default: string | null
  type: 'secret' | 'variable'
  /** Shaped like a database URL: iii cloud makes the database. */
  database: boolean
  containers: string[]
  line: number | null
  in_cloud: 'set' | 'unset' | 'unknown'
  /** Only a set variable's value in iii cloud; never a secret's. */
  value: string | null
  updated_at: string | null
  asked: { request_id: string; description: string } | null
  /** In preselect order: the UI takes the first with `preselect: true`. A row set in iii cloud, a `database` row or an `invalid_name` row has none. */
  suggestions: Suggestion[]
  /** Why iii cloud can't store this name (lower case, say): nothing is preselected and `values::save` refuses the row with `INVALID_REQUEST`. */
  invalid_name?: string
  /** Whether the run here has a value: in its `.env` (`set`), from an env file of your folder (`from_file`, see `here_file`), nowhere (`unset`, `LOCAL_VARIABLE_UNSET`), or none needed: a default, or a name the compose file doesn't read (`not_needed`). */
  here?: 'set' | 'from_file' | 'unset' | 'not_needed'
  /** With `here: 'from_file'`: the file, such as `api/.env.test`. */
  here_file?: string | null
  /** The person skipped it for now (`values::skip`): the run here gives it `""`, and its problems say `skipped`. */
  skipped?: boolean
}

export type Suggestion =
  | { source: 'env_file'; file: string; preselect: boolean; local: boolean }
  /** `preselect` is false only on a row set in iii cloud, a `database` row or an `invalid_name` row. */
  | { source: 'env_url'; url: string; preselect: boolean }
  | { source: 'copy'; environment_id: string; environment: string; preselect: false }

/** One name of `iii-cloud::values::save`'s `items`: where its value comes from. */
export type SaveItem =
  | { name: string; from: 'typed'; type: 'secret' | 'variable'; value: string }
  | { name: string; from: 'env_file'; type: 'secret' | 'variable'; file: string }
  /** Written as a variable. */
  | { name: string; from: 'env_url' }
  /** Written as the variable `""`. */
  | { name: string; from: 'leave_empty' }
  | { name: string; from: 'copy'; environment_id: string }

/** What `iii-cloud::values::skip` answers: the environment's skipped names after the call, sorted. */
export type ValuesSkipped = { environment_id: string; names: string[] }

export type ValuesSaved = {
  environment_id: string
  results: { name: string; ok: boolean; error?: WorkerError }[]
}

/** `iii-cloud::override::choose`: the page's own. `file`: the file's value wins at deploy (the deploy already does
    that); `console`: the person wants the console's kept, which takes a change to the file. */
export type OverrideChoice = 'file' | 'console'

/** `iii-cloud::config::import`: the page's own. Copies the compose file's local config folder (the `configuration`
    worker's `fs` adapter `directory`, from the person's folder) into the environment's console config. A dry run
    writes nothing; applying needs the dry run's `version`. */
export type ConfigImport = { environment_id: string; dry_run: boolean; expected_version?: number | null }

export type ConfigImported = {
  environment_id: string
  /** The config folder, from the repository root (`api/config`). */
  directory: string
  /** The config version the dry run read (send it back as `expected_version`), or the one the apply made. */
  version: number
  /** False for a dry run. */
  applied: boolean
  /** iii cloud has unapplied console edits for this environment: applying is refused until they're applied or dropped there. */
  draft_changed: boolean
  workers: ConfigImportWorker[]
  /** Files of the folder not copied, and why. */
  skipped: { file: string; reason: ConfigSkipReason }[]
}

export type ConfigImportWorker = {
  /** The container key iii cloud keeps the config by. */
  worker: string
  /** The file, from the repository root. */
  file: string
  /** The fields copied, as dotted keys. `value` only in a dry run's preview; null once applied. Lists stay JSON. */
  fields: { key: string; value: unknown }[]
  /** Secret-looking keys or credential-looking values, by key only: set a secret and write `${NAME}` in iii cloud. */
  left_out: string[]
  /** Keys iii cloud or the compose file set, which the console can't change. */
  set_elsewhere: string[]
}

/** `not_a_worker`: no container iii cloud runs has that config entry; `not_a_plain_file`: a link or outside the
    repository; `too_large`: over 1 MiB; `unreadable`: not a configuration entry (`id`, `name`, `description`, `value`). */
export type ConfigSkipReason = 'not_a_worker' | 'not_a_plain_file' | 'too_large' | 'unreadable'
