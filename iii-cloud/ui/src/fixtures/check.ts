// CheckStage boards (U2): deploy-checking, deploy-problems, deploy-fixing, setup-fixing, deploy-passed and the state-* panels;
// setup-fixing-live is the live release-control · test screen of 26/11 (CHECKSTAGE-PLAN).
// One story on `dev`, a direct environment: the person's commits e41a7c3 are checked, the agent fixes them in
// its copy (3f1b2aa, then 9c04e7f) and the last check passes.

import type {
  AgentRun,
  Check,
  CheckSummary,
  ConfigImported,
  Deploy,
  HandoffBrief,
  Problem,
  Project,
  Run,
  SecretRequest,
  Stage,
  Status,
} from '../types'
import { agent, type Board, base, DEPLOY_BASE, ENV, NOW, project, request, run, T } from './base'

export const DEV = { id: 'e2f4a6c8-1b3d-4f5a-9c7e-0d2b4f6a8c10', name: 'dev', production: false, branch: 'dev' }

/** A full sha from its short form, so `sha7` gives the board's. */
const sha = (seven: string) => seven.padEnd(40, '0')
export const SHA = {
  /** The person's last commit: what came into the agent's copy (`workspace.from_head`). */
  mine: sha('e41a7c3'),
  pinned: sha('3f1b2aa'),
  last: sha('9c04e7f'),
  /** `dev`'s head when it was checked (`check.base`). */
  tip: sha('86c49e1'),
  /** Someone else's push to `dev` after the check. */
  pushed: sha('1f22ab0'),
  /** A commit the person made after the check. */
  later: sha('2be71d0'),
}

const checkout: Project['checkout'] = {
  branch: 'billing',
  head: SHA.mine,
  changed_files: [],
  ahead: 3,
  not_served: [
    { sha: SHA.mine, message: 'Search worker' },
    { sha: sha('5d1c0b9'), message: 'Billing worker' },
  ],
}

const devProject = (over: Partial<Project> = {}, head = SHA.tip): Project => ({
  ...project(true),
  default_branch: 'main',
  checkout,
  environment: { ...DEV, manifest: 'api/worker-compose-prod.yaml', url: 'https://dev.registry.iii.dev' },
  environments: [ENV, DEV],
  workspace: {
    mode: 'worktree',
    path: '~/workspaces/registry-iii-cloud',
    branch: 'iii-cloud/ready-dev',
    from_head: SHA.mine,
    source_sha: SHA.mine,
  },
  cloud_projects: [
    {
      id: 'p1',
      name: 'registry',
      slug: 'registry',
      environments: [
        { ...ENV, head: null },
        { ...DEV, head },
      ],
    },
  ],
  ...over,
})

const problem = (
  id: string,
  source: Problem['source'],
  code: string,
  owner: Problem['owner'],
  title: string,
  detail: string,
): Problem => ({
  id,
  source,
  code,
  cloud_code: null,
  severity: 'block',
  owner,
  title,
  detail,
  file: null,
  line: null,
  excerpt: null,
  fix: null,
})

export const pin = problem(
  'pin',
  'cloud_plan',
  'WORKER_NOT_PINNED',
  'agent',
  'The new search worker has no pinned version',
  'The cloud builds only pinned versions, so the build would stop.',
)
export const stripe = problem(
  'stripe',
  'cloud_plan',
  'LOCAL_VARIABLE_UNSET',
  'person',
  'Stripe secret key for dev',
  'STRIPE_SECRET_KEY: your new billing worker reads it.',
)
export const rights = problem(
  'rights',
  'check_here',
  'MIGRATION_NEEDS_SUPERUSER',
  'agent',
  'A database migration needs rights the cloud doesn’t give',
  '0008_trgm.sql installs pg_stat_statements, which only an administrator can install. Found by running it here.',
)

const stage = (name: Stage['name'], status: Stage['status'], detail: string | null, ms: number | null): Stage => ({
  name,
  status,
  detail,
  duration_ms: ms,
})
const after = (failed: Stage['name'][]) =>
  (['compose_up', 'workers', 'routes', 'healthcheck'] as const).map((n) =>
    stage(n, failed.length ? 'skipped' : 'pending', null, null),
  )
const runningStages = [
  stage('install', 'passed', 'pnpm install --frozen-lockfile', 6_100),
  stage('build', 'passed', 'cloud.build', 9_000),
  stage('release', 'running', 'db:migrate', null),
  ...after([]),
]
const releaseFailed = [
  stage('install', 'passed', 'pnpm install --frozen-lockfile', 6_100),
  stage('build', 'passed', 'cloud.build', 9_000),
  stage('release', 'failed', 'db:migrate', 2_000),
  ...after(['release']),
]
export const RELEASE_LOG = [
  'applying migration 0008_trgm.sql',
  'ERROR: permission denied to create extension "pg_stat_statements"',
  'HINT: Must be superuser to create this extension.',
]

const runOf = (check_id: string, commit: string, status: Run['status'], over: Partial<Run> = {}): Run => ({
  ...run,
  id: `run-${check_id}`,
  check_id,
  commit,
  status,
  started_at: status === 'running' ? new Date(NOW - 11_000).toISOString() : T(2),
  finished_at: status === 'running' ? null : new Date(Date.parse(T(2)) + 41_000).toISOString(),
  stages: status === 'running' ? runningStages : status === 'failed' ? releaseFailed : run.stages,
  log_tail: status === 'failed' ? RELEASE_LOG : [],
  error:
    status === 'failed' ? { stage: 'release', code: 'RELEASE_FAILED', message: 'db:migrate exited with 1.' } : null,
  ...over,
})

const blocked: Check['plan'] = {
  status: 'blocked',
  fingerprint: null,
  digest: null,
  blocking: 2,
  warnings: 0,
  at_least: false,
  confirmations: { resources: [], overrides: [] },
  web_candidates: null,
  error: null,
  summary: null,
}
const clean: Check['plan'] = {
  ...blocked,
  status: 'clean',
  blocking: 0,
  summary: { workers: 9, groups: ['app', 'web', 'jobs'] },
}

const devCheck = (id: string, over: Partial<Check>): Check => ({
  id,
  environment_id: DEV.id,
  commit: SHA.mine,
  base: SHA.tip,
  checked_at: T(1),
  duration_ms: 3_000,
  plan: blocked,
  here: { problems: 0, iii_version: '0.27.0', accepts_cloud_block: true },
  problems: [pin, stripe],
  run: null,
  ready: false,
  report: '',
  manifest: 'api/worker-compose-prod.yaml',
  handoff_id: null,
  commits: [],
  ...over,
})

export const summaryOf = (c: Check, problems: number): CheckSummary => ({
  id: c.id,
  environment_id: c.environment_id,
  commit: c.commit,
  checked_at: c.checked_at,
  duration_ms: c.duration_ms,
  ready: c.ready,
  problems,
  handoff_id: c.handoff_id ?? null,
  manifest: c.manifest ?? null,
})

// The person's check, while it runs and once it found three problems.
export const checking = devCheck('chk-d1', { checked_at: T(0), run: runOf('chk-d1', SHA.mine, 'running') })
export const found = devCheck('chk-d1', {
  checked_at: T(9),
  problems: [pin, stripe, rights],
  here: { problems: 1, iii_version: '0.27.0', accepts_cloud_block: true },
  run: runOf('chk-d1', SHA.mine, 'failed'),
})
// The agent's checks: pinned (one problem left), then its latest fix, running, and the one that passed.
const pinned = devCheck('chk-d2', {
  commit: SHA.pinned,
  checked_at: T(6),
  handoff_id: 'ho-d',
  plan: clean,
  problems: [rights],
  run: runOf('chk-d2', SHA.pinned, 'failed'),
})
export const rerun = devCheck('chk-d3', {
  commit: SHA.last,
  checked_at: T(0),
  handoff_id: 'ho-d',
  plan: clean,
  problems: [rights],
  run: runOf('chk-d3', SHA.last, 'running'),
})
export const passed = devCheck('chk-d4', {
  commit: SHA.last,
  checked_at: T(1),
  handoff_id: 'ho-d',
  plan: clean,
  problems: [],
  run: runOf('chk-d4', SHA.last, 'passed'),
  ready: true,
})

const brief = (p: Problem) => ({ title: p.title, file: p.file, line: p.line, detail: p.detail, fix: p.fix })
export const fixBrief: HandoffBrief = {
  handoff_id: 'ho-d',
  kind: 'ready',
  repository: 'iii-hq/registry',
  project: 'registry · dev',
  worktree: { path: '~/workspaces/registry-iii-cloud', branch: 'iii-cloud/ready-dev' },
  checkout_path: '/home/you/workspaces/registry',
  check_id: 'chk-d1',
  commit: SHA.mine,
  agent_problems: [pin, rights].map(brief),
  person_problems: [stripe].map(brief),
  deploy: null,
  next_step: '',
}
export const fixer: AgentRun = {
  ...agent('working'),
  handoff_id: 'ho-d',
  kind: 'ready',
  started_at: T(4),
  working_on: ['rights'],
  current: 'Removing pg_stat_statements from 0008_trgm.sql',
  note: 'Keeping pg_trgm, which the app’s database user can install',
}
export const stripeAsk: SecretRequest = {
  ...request(
    'STRIPE_SECRET_KEY',
    'ask',
    'waiting',
    'Your new billing worker reads it. Found by iii cloud. Saved to dev only.',
  ),
  environment_id: DEV.id,
}
const stripeSaved: SecretRequest = { ...stripeAsk, state: 'set', cloud: 'set', local: 'set', resolved_at: T(3) }

const dev: Status = { ...base, project: devProject(), check: checking, checks: [summaryOf(checking, 2)] }
const fixingNow: Status = {
  ...dev,
  check: rerun,
  checks: [summaryOf(rerun, 1), summaryOf(pinned, 1), summaryOf(found, 3)],
  agent: fixer,
  handoff_brief: fixBrief,
  secrets: [stripeSaved],
}
const passedNow: Status = {
  ...fixingNow,
  check: passed,
  checks: [summaryOf(passed, 0), summaryOf(rerun, 1), summaryOf(pinned, 1), summaryOf(found, 3)],
  agent: { ...fixer, state: 'done', working_on: [], current: null, note: null },
}
const consoleDeploy: Deploy = {
  ...DEPLOY_BASE,
  id: 'dep-dev-6',
  number: 6,
  environment_id: DEV.id,
  commit: { sha: SHA.pushed, message: 'Raise the search timeout' },
  type: 'console',
  from_console: true,
  pr: null,
  approved_by_worker: false,
  checked_ready: false,
  created_at: T(2),
  check_id: null,
  console_url: 'https://cloud.iii.dev/iii/registry/dev/deployments/6',
}
const signIn = problem(
  'signin',
  'cloud_plan',
  'SIGN_IN_REQUIRED',
  'person',
  'You need to sign in to iii cloud again',
  'iii cloud no longer accepts your sign-in.',
)

/* ── setup-fixing-live: release-control · test while the agent fixes (the live screen of 26/11) ── */

export const TEST_ENV = { id: '4c7e9a12-6b3d-4e8f-a1c5-9d2b7f0e3a64', name: 'test', production: false, branch: 'test' }
const RC_FILE = 'api/worker-compose-prod.yaml'
const RC_HEAD = sha('1647712')
/** The 21 variables the file defaults to empty, lines 81–101, in file order. */
export const RC_VARIABLES = [
  'APP_BASE_URL',
  'DATABASE_URL',
  'GH_APP_CLIENT_ID',
  'GH_APP_CLIENT_SECRET',
  'GH_APP_ID',
  'GH_APP_INSTALLATION_ID',
  'GH_APP_PRIVATE_KEY',
  'GH_ORG',
  'GH_REPO',
  'GH_WEBHOOK_SECRET',
  'HARNESS_E2E_OIDC_AUDIENCE',
  'HARNESS_E2E_SHADOW_ENABLED',
  'HARNESS_E2E_WORKFLOW_FILE',
  'HARNESS_E2E_WORKFLOW_REF',
  'LOG_LEVEL',
  'RELEASE_NOTES_URL',
  'SENTRY_DSN',
  'SESSION_SECRET',
  'SLACK_BOT_TOKEN',
  'SLACK_CHANNEL',
  'SLACK_NOTIFICATIONS_ENABLED',
]

/** A cloud plan issue as the worker records it: "See file:line. (at path)", with five lines around it. */
const planIssue = (
  id: string,
  code: string,
  severity: Problem['severity'],
  owner: Problem['owner'],
  title: string,
  line: number,
  at: string,
  lines: string[],
): Problem => ({
  id,
  source: 'cloud_plan',
  code,
  cloud_code: code,
  severity,
  owner,
  title,
  detail: `See ${RC_FILE}:${line}. (at ${at})`,
  file: RC_FILE,
  line,
  excerpt: { start_line: line - 2, lines },
  fix: null,
})
const envLines = (i: number) =>
  [-2, -1, 0, 1, 2]
    .map((d) => RC_VARIABLES[i + d])
    .map((n) => (n ? `      ${n}: \${${n}:-}` : '    restart: unless-stopped'))
const outdated = (id: string, code: string, container: string, line: number): Problem =>
  planIssue(
    id,
    code,
    code === 'QUEUE_WORKER_OUTDATED' ? 'warn' : 'block',
    'agent',
    code === 'QUEUE_WORKER_OUTDATED' ? 'Queue worker outdated' : 'Worker outdated',
    line,
    `containers.${container}.worker`,
    [`  ${container}:`, '    # pinned by the lock', `    worker: iii-hq/${container}`, '    version: 0.3.1', ''],
  )
const here = (id: string, code: string, title: string, detail: string, file: string | null = null): Problem => ({
  ...problem(id, 'check_here', code, 'agent', title, detail),
  file,
})

export const rcEngine = {
  ...planIssue('rc-engine', 'ENGINE_SECTION_IGNORED', 'info', 'none', 'Engine section ignored', 21, 'engine', [
    'stop_timeout: 10s',
    '',
    'engine:',
    '  url: ws://127.0.0.1:4911',
    '  workers:',
  ]),
  detail: 'iii cloud runs the engine. The same file still runs locally. (at engine)',
}
export const rcConfig: Problem = {
  ...planIssue(
    'rc-config',
    'LOCAL_CONFIG_NOT_SET',
    'warn',
    'person',
    "The workers' config isn't in iii cloud yet",
    28,
    'engine.workers.configuration.adapter.config.directory',
    [
      '        name: fs',
      '        config:',
      '          directory: ./config',
      '    iii-worker-manager:',
      '      host: 0.0.0.0',
    ],
  ),
  detail:
    "Your app file reads config from ./config, which isn't in the repository. Values you save in the worker's config in Settings apply instead. (at engine.workers.configuration.adapter.config.directory)",
}
export const rcVariables = RC_VARIABLES.map((name, i) =>
  planIssue(
    `rc-var-${name}`,
    'EMPTY_DEFAULT_VARIABLE',
    'warn',
    'person',
    'A variable defaults to empty',
    81 + i,
    `containers.worker-release-control.environment.${name}`,
    envLines(i),
  ),
)
export const rcBun: Problem = {
  ...problem(
    'rc-bun',
    'check_here',
    'BUN_NO_LOCK',
    'person',
    'Runs with bun, but the repository has only a pnpm-lock.yaml lock',
    'The cloud installs bun projects from bun.lock, so nothing would be installed. Run it with node and keep the pnpm lock, or commit a bun.lock.',
  ),
}
/** Four outdated workers, two of them the same line 52 under two codes. */
export const rcOutdated = [
  outdated('rc-http', 'WORKER_OUTDATED', 'http', 40),
  outdated('rc-cron', 'WORKER_OUTDATED', 'cron', 46),
  outdated('rc-queue-q', 'QUEUE_WORKER_OUTDATED', 'queue', 52),
  outdated('rc-queue', 'WORKER_OUTDATED', 'queue', 52),
]
export const rcSdk = here(
  'rc-sdk',
  'SDK_TOO_OLD',
  'iii-sdk 0.11.0 is older than 0.26',
  'The deploy would go green and the http group would serve no route.',
)
const rcMigrations: Problem = {
  ...here(
    'rc-migrate',
    'RELEASE_IN_CI',
    'Migrations run in your CD, not in the cloud',
    '.github/workflows/cd.yml runs db:migrate; it moves to cloud.release.',
    '.github/workflows/cd.yml',
  ),
  severity: 'warn',
}
const rcHealth: Problem = {
  ...here(
    'rc-health',
    'HEALTHCHECK_MISSING',
    'No healthcheck',
    'Without one, a deploy is Ready even when nothing answers.',
  ),
  severity: 'warn',
}
const rcProblems: Problem[] = [rcEngine, rcConfig, ...rcVariables, ...rcOutdated, rcBun, rcSdk, rcMigrations, rcHealth]
const RUN_LOG = [
  '[engine]   1 of 1 workers registered',
  '[http]     iii-sdk 0.11.0: no route registered under /api',
  'GET /api/health 404 3ms',
]
export const rcCheck: Check = {
  id: 'chk-rc1',
  environment_id: TEST_ENV.id,
  commit: RC_HEAD,
  base: RC_HEAD,
  checked_at: T(7),
  duration_ms: 9_400,
  plan: { ...blocked, blocking: 3, warnings: 24, summary: { workers: 1, groups: ['app'] } },
  here: { problems: 4, iii_version: '0.27.0', accepts_cloud_block: true },
  problems: rcProblems,
  run: {
    ...runOf('chk-rc1', RC_HEAD, 'failed'),
    stages: [
      stage('install', 'passed', 'pnpm install --frozen-lockfile', 14_000),
      stage('build', 'passed', 'cloud.build', 6_000),
      stage('release', 'passed', 'db:migrate', 3_000),
      stage('compose_up', 'passed', 'worker-release-control', 9_000),
      stage('workers', 'passed', null, 2_000),
      stage('routes', 'failed', null, 1_000),
      stage('healthcheck', 'skipped', null, null),
    ],
    log_tail: RUN_LOG,
    error: { stage: 'routes', code: 'NO_ROUTE', message: 'The http group serves no route.' },
  },
  ready: false,
  report: '',
  manifest: RC_FILE,
  handoff_id: null,
  commits: [],
}
const rcBrief: HandoffBrief = {
  handoff_id: 'ho-rc',
  kind: 'ready',
  repository: 'iii-hq/release-control',
  project: 'release-control · test',
  worktree: { path: '~/workspaces/release-control-iii-cloud', branch: 'iii-cloud/ready-test' },
  checkout_path: '/home/you/workspaces/release-control',
  check_id: rcCheck.id,
  commit: RC_HEAD,
  agent_problems: rcProblems.filter((p) => p.owner === 'agent').map(brief),
  person_problems: rcProblems.filter((p) => p.owner === 'person').map(brief),
  deploy: null,
  next_step: '',
}
const rcProject: Project = {
  path: '/home/you/workspaces/release-control',
  repository: 'iii-hq/release-control',
  checkout: { branch: 'main', head: RC_HEAD, changed_files: [], ahead: 0 },
  team: base.project?.team ?? null,
  teams: [],
  cloud_project: { id: 'p-rc', name: 'release-control', slug: 'release-control' },
  default_branch: 'main',
  environment: { ...TEST_ENV, manifest: RC_FILE, url: null },
  environments: [TEST_ENV],
  importable: null,
  github_access: true,
  workspace: {
    mode: 'worktree',
    path: '~/workspaces/release-control-iii-cloud',
    branch: 'iii-cloud/ready-test',
    from_head: RC_HEAD,
    source_sha: RC_HEAD,
  },
  cloud_projects: [
    { id: 'p-rc', name: 'release-control', slug: 'release-control', environments: [{ ...TEST_ENV, head: RC_HEAD }] },
  ],
}
export const rcAgent: AgentRun = {
  ...agent('working'),
  handoff_id: 'ho-rc',
  kind: 'ready',
  started_at: T(6),
  last_activity_at: new Date(NOW - 36_000).toISOString(),
  steps: 11,
  current: 'Find the cloud compose parser and web prefix examples in installed worker sources.',
  note: 'I am preparing a separate Node manifest and preserving the ECS deployment files.',
  working_on: rcProblems.filter((p) => p.owner === 'agent').map((p) => p.id),
}
export const rcFixing: Status = {
  ...base,
  project: rcProject,
  check: rcCheck,
  checks: [summaryOf(rcCheck, 30)],
  agent: rcAgent,
  handoff_brief: rcBrief,
  secrets: [],
}

/** P3: two names the run here has no value for, in neither its .env nor the folder's .env files. */
export const RC_HERE_UNSET = ['SESSION_SECRET', 'LOG_LEVEL']
const rcHere = RC_HERE_UNSET.map((name) =>
  problem(
    `here-${name}`,
    'check_here',
    'LOCAL_VARIABLE_UNSET',
    'person',
    `${name} has no value here`,
    "The run here needs it: it is not in the run's .env, nor in your folder's .env files.",
  ),
)
export const rcFixingHere: Status = {
  ...rcFixing,
  check: { ...rcCheck, problems: [...rcProblems, ...rcHere] },
  handoff_brief: { ...rcBrief, person_problems: [...rcBrief.person_problems, ...rcHere.map(brief)] },
}

/** SKIP-VALUES: two optional names skipped for now, as the worker answers after `values::skip` (info, skipped). */
export const RC_SKIPPED = ['LOG_LEVEL', 'SENTRY_DSN']
const skippedVar = (p: Problem, over: Partial<Problem>): Problem =>
  RC_SKIPPED.some((n) => p.id === `rc-var-${n}`) ? { ...p, skipped: true, ...over } : p
export const rcSkipped: Status = {
  ...rcFixing,
  skipped_values: RC_SKIPPED,
  check: { ...rcCheck, problems: rcProblems.map((p) => skippedVar(p, { severity: 'info' })) },
}
/** The same names as SECRET_MISSING (`${X}`): skipped, and iii cloud still blocks on them. */
export const rcSecretSkipped: Status = {
  ...rcFixing,
  skipped_values: RC_SKIPPED,
  check: {
    ...rcCheck,
    problems: rcProblems
      .map((p) =>
        p.code === 'EMPTY_DEFAULT_VARIABLE'
          ? { ...p, code: 'SECRET_MISSING', cloud_code: 'SECRET_MISSING', severity: 'block' as const }
          : p,
      )
      .map((p) => skippedVar(p, { detail: 'iii cloud needs a value to deploy, even an empty one.' })),
  },
}

const deploy = { at: 'deploy', env: DEV.id } as const

/** O1: dev's console serves the web container to the team; the app file opens it to the public. */
const WEB_CONSOLE = { serve: 'container', container: 'web', port: 3000, access: 'team' }
export const webOverride = { setting: 'web', console: WEB_CONSOLE, file: { ...WEB_CONSOLE, access: 'public' } }
/** O2: the workers' config warning (./config isn't in the repository), for a board of another environment. */
export const devConfig: Problem = { ...rcConfig, id: 'dev-config' }
const overridden = (c: Check, overrides: (typeof webOverride)[]): Check => ({
  ...c,
  plan: { ...c.plan, confirmations: { resources: [], overrides } },
})
const f = (key: string, value: unknown) => ({ key, value })
/**
 * O2: what `config::import` previews for release-control · test (the live dry run of 26/11): container keys, not
 * file stems; http's host and port are set by iii cloud; iii-observability.yaml is no container's; nothing secret.
 */
export const configPreview: ConfigImported = {
  environment_id: TEST_ENV.id,
  directory: 'api/config',
  version: 3,
  applied: false,
  draft_changed: false,
  workers: [
    {
      worker: 'cron',
      file: 'api/config/cron.yaml',
      fields: [f('adapter.name', 'kv')],
      left_out: [],
      set_elsewhere: [],
    },
    {
      worker: 'http',
      file: 'api/config/http.yaml',
      fields: [
        f('default_timeout', 30000),
        f('concurrency_request_limit', 1024),
        f('body_limit', '10mb'),
        f('cors.allowed_origins', ['https://release.iii.dev']),
        f('cors.allowed_methods', ['GET', 'POST', 'PUT', 'DELETE', 'OPTIONS']),
      ],
      left_out: [],
      set_elsewhere: ['host', 'port'],
    },
    {
      worker: 'queue',
      file: 'api/config/queue.yaml',
      fields: [
        f('adapter.name', 'builtin'),
        f('adapter.config.store_method', 'file_based'),
        f('queue_configs.default.type', 'standard'),
        f('queue_configs.default.concurrency', 10),
        f('queue_configs.default.max_retries', 3),
        f('queue_configs.default.backoff_ms', 1000),
        f('queue_configs.default.poll_interval_ms', 100),
        f('queue_configs.release.type', 'fifo'),
        f('queue_configs.release.concurrency', 1),
        f('queue_configs.release.max_retries', 5),
        f('queue_configs.release.backoff_ms', 5000),
        f('queue_configs.release.message_group_field', 'environment_id'),
      ],
      left_out: [],
      set_elsewhere: [],
    },
  ],
  skipped: [{ file: 'api/config/iii-observability.yaml', reason: 'not_a_worker' }],
}

/**
 * registry · dev live, 26/11: iii cloud accepts the commit (two warnings, a note, four skipped optional names) and
 * the run here never ran; the agent is done. Nothing blocks: the run here is what is left.
 */
const runPendingCheck = devCheck('chk-d6', {
  commit: SHA.last,
  plan: { ...clean, warnings: 2 },
  problems: [
    devConfig,
    planIssue(
      'dev-sqlite',
      'SQLITE_DEFAULT_REPLACED',
      'warn',
      'person',
      'SQLite is replaced by Postgres',
      33,
      'containers.api.environment.DATABASE_URL',
      ['      DATABASE_URL: sqlite://data.db'],
    ),
    rcEngine,
    ...rcVariables.slice(0, 4).map((p) => ({ ...p, severity: 'info' as const, skipped: true })),
  ],
  run: null,
  ready: false,
})

export type CheckBoardId =
  | 'deploy-checking'
  | 'deploy-problems'
  | 'deploy-fixing'
  | 'setup-fixing'
  | 'deploy-passed'
  | 'deploy-decision'
  | 'deploy-open-problem'
  | 'state-docker'
  | 'state-stalled'
  | 'state-stopped'
  | 'state-busy'
  | 'state-override'
  | 'state-override-chosen'
  | 'state-moved'
  | 'state-signedout'
  | 'state-outdated'
  | 'setup-fixing-live'
  | 'setup-fixing-here'
  | 'setup-fixing-skipped'
  | 'setup-fixing-secret-skipped'
  | 'setup-config-override'
  | 'setup-config-skipped'
  | 'deploy-run-pending'

export const boards: Record<CheckBoardId, Board> = {
  'deploy-checking': { status: dev, context: deploy },
  'deploy-problems': {
    status: { ...dev, check: found, checks: [summaryOf(found, 3)], secrets: [stripeAsk] },
    context: deploy,
  },
  'deploy-fixing': { status: fixingNow, context: deploy },
  'setup-fixing': {
    status: { ...fixingNow, check: { ...rerun, problems: [rights, stripe] }, secrets: [stripeAsk] },
    context: { at: 'setup' },
  },
  'deploy-passed': { status: passedNow, context: deploy },
  // Check2: the secret waits for the person, a console setting differs from the compose file, the agent fixes the rest.
  'deploy-decision': {
    status: {
      ...fixingNow,
      check: { ...overridden(rerun, [webOverride]), problems: [rights, stripe] },
      secrets: [stripeAsk],
    },
    context: deploy,
  },
  // A problem only the person can solve, with the check's own words: what, why and the fix.
  'deploy-open-problem': {
    status: {
      ...dev,
      check: {
        ...found,
        problems: [
          rights,
          {
            ...problem(
              'credential',
              'cloud_plan',
              'LITERAL_CREDENTIAL',
              'person',
              'A credential is written in your compose file',
              'The key is committed with the file, so anyone who reads the repository can read it.',
            ),
            file: 'api/worker-compose-prod.yaml',
            line: 41,
            fix: 'Move the key to a secret and have the file read that secret.',
          },
        ],
      },
      checks: [summaryOf(found, 2)],
    },
    context: deploy,
  },
  'state-docker': {
    status: {
      ...dev,
      check: {
        ...found,
        problems: [pin, stripe],
        run: runOf('chk-d1', SHA.mine, 'failed', {
          error: { stage: 'release', code: 'DOCKER_MISSING', message: 'docker could not start Postgres' },
          log_tail: [],
        }),
      },
    },
    context: deploy,
  },
  'state-stalled': {
    status: { ...fixingNow, agent: { ...fixer, state: 'stalled', last_activity_at: T(10), working_on: [] } },
    context: deploy,
  },
  'state-stopped': {
    status: {
      ...fixingNow,
      check: { ...rerun, run: runOf('chk-d3', SHA.last, 'failed') },
      agent: { ...fixer, state: 'stopped', working_on: [], current: null },
    },
    context: deploy,
  },
  'state-busy': { status: { ...passedNow, deploys: [consoleDeploy] }, context: deploy },
  'state-override': {
    status: {
      ...passedNow,
      check: {
        ...passed,
        plan: {
          ...clean,
          confirmations: {
            resources: [],
            overrides: [{ ...webOverride, file: { ...WEB_CONSOLE, container: 'site', port: 8080 } }],
          },
        },
      },
    },
    context: deploy,
  },
  'state-override-chosen': {
    status: { ...passedNow, check: overridden(passed, [webOverride]), override_choices: { web: 'file' } },
    context: deploy,
  },
  'state-moved': { status: { ...passedNow, project: devProject({}, SHA.pushed) }, context: deploy },
  'state-signedout': {
    status: {
      ...dev,
      check: devCheck('chk-d5', {
        plan: { ...blocked, status: 'unavailable', blocking: 0 },
        problems: [signIn],
        run: runOf('chk-d5', SHA.mine, 'passed'),
      }),
    },
    context: deploy,
  },
  'state-outdated': {
    status: {
      ...passedNow,
      project: devProject({
        checkout: {
          ...checkout,
          head: SHA.later,
          not_served: [{ sha: SHA.later, message: 'Search page' }, ...(checkout.not_served ?? [])],
        },
      }),
    },
    context: deploy,
  },
  'setup-fixing-live': { status: rcFixing, context: { at: 'setup' } },
  'setup-fixing-here': { status: rcFixingHere, context: { at: 'setup' } },
  'setup-fixing-skipped': { status: rcSkipped, context: { at: 'setup' } },
  'setup-fixing-secret-skipped': { status: rcSecretSkipped, context: { at: 'setup' } },
  // O1/O2: the stuck screen of 26/11: dev's console web setting against the file, and the workers' config.
  'setup-config-override': {
    status: { ...rcFixing, check: overridden(rcCheck, [webOverride]) },
    context: { at: 'setup' },
    configImport: configPreview,
  },
  'deploy-run-pending': {
    status: { ...passedNow, check: runPendingCheck, checks: [summaryOf(runPendingCheck, 0)] },
    context: deploy,
  },
  // O2: skipped for now, then checked again: the check marks it skipped, as info.
  'setup-config-skipped': {
    status: {
      ...rcFixing,
      check: {
        ...rcCheck,
        problems: rcCheck.problems.map((p) =>
          p.code === 'LOCAL_CONFIG_NOT_SET' ? { ...p, skipped: true, severity: 'info' as const } : p,
        ),
      },
      skipped_problems: ['LOCAL_CONFIG_NOT_SET'],
    },
    context: { at: 'setup' },
  },
}
