// Builders every board starts from. A board's `status` is what `iii-cloud::status`
// returns; `ui/fixtures/<board>/state.json` holds the same record as a store file
// for the visual check (`node fixtures/sync.mjs`).

import type { Route } from '../route'
import type {
  AgentRun,
  Check,
  ConfigImported,
  Connection,
  Deploy,
  PullRequest,
  Run,
  SecretRequest,
  Status,
  Values,
} from '../types'

/**
 * One board: the status, the route it opens at, and
 * `cloud`, the bodies `iii-cloud::api` answers, by full GET path (`apiPath`), with `?query` when it has one.
 */
export type Board = {
  status: Status
  context?: Route
  width?: number
  dialog?: boolean
  cloud?: Record<string, unknown>
  /** What `iii-cloud::values` answers. */
  values?: Values
  /** What `iii-cloud::config::import` answers (the dry run's preview). */
  configImport?: ConfigImported
}

// Relative to load time: "2m ago" and "expires in 9:41" read the same whenever it runs.
export const NOW = Date.now()
export const T = (min: number) => new Date(NOW - min * 60_000).toISOString()

export const COMMIT = '3c4d2e0a91b7f5d6c8e2a4b0917d3e5f6a8c1b2d'
export const FINGERPRINT = 'e1b7ccfe5a0d49c3b6f28a71d4e9035c8b16f2a7'
export const TEAM = { id: '5b0d6f2e-0c1a-4e3b-9a77-1d2c3b4a5f60', name: 'iii', slug: 'iii' }
export const ENV = { id: '9a1c2b3d-4e5f-4a6b-8c7d-0e1f2a3b4c5d', name: 'production', production: true, branch: 'main' }

export const connected: Connection = {
  status: 'connected',
  method: 'browser',
  cloud_url: 'https://cloud.iii.dev',
  user: { id: 'u1', email: 'you@example.com', name: null },
  teams: [{ ...TEAM, role: 'Owner' }],
  api_key: null,
  device: null,
  expires_at: new Date(NOW + 90 * 86_400_000).toISOString(),
  error: null,
}

export const waiting: Connection = {
  ...connected,
  status: 'waiting',
  user: null,
  teams: [],
  expires_at: null,
  device: {
    user_code: 'WXYZ-1234',
    verification_uri: 'https://cloud.iii.dev/device',
    expires_at: new Date(NOW + (9 * 60 + 41) * 1000).toISOString(),
  },
}

export const signedOut: Connection = { ...waiting, status: 'signed_out', method: null, device: null }

export const project = (workspace: boolean): NonNullable<Status['project']> => ({
  path: '/home/you/workspaces/registry',
  repository: 'iii-hq/registry',
  checkout: { branch: 'main', head: COMMIT, changed_files: ['api/worker-compose-prod.yaml', 'README.md'], ahead: 0 },
  team: TEAM,
  teams: [],
  cloud_project: { id: 'p1', name: 'registry', slug: 'registry' },
  environment: { ...ENV, manifest: 'api/worker-compose-prod.yaml', url: null },
  environments: [ENV],
  importable: null,
  github_access: true,
  workspace: workspace
    ? { mode: 'worktree', path: '~/workspaces/registry-iii-cloud', branch: 'iii-cloud/ready-production' }
    : null,
})

export const problems: Check['problems'] = [
  {
    id: 'p1',
    source: 'cloud_plan',
    code: 'LOCK_MISSING',
    cloud_code: null,
    severity: 'block',
    owner: 'agent',
    title: 'Lock missing',
    detail: 'The compose file has no lock beside it: api/worker-compose-prod.lock.',
    file: null,
    line: null,
    excerpt: null,
    fix: 'Generate the lock and commit it next to the compose file.',
  },
  {
    id: 'p2',
    source: 'cloud_plan',
    code: 'MISSING_WORKER_DIRECTORY',
    cloud_code: null,
    severity: 'block',
    owner: 'person',
    title: 'http and storage point at folders only the Dockerfile downloads',
    detail: 'http and storage point at folders only the Dockerfile downloads (lines 21 and 32).',
    file: null,
    line: null,
    excerpt: null,
    fix: 'Keep path:// and version the two folders, or switch them to a registry name.',
  },
  {
    id: 'p3',
    source: 'check_here',
    code: 'SDK_OLD',
    cloud_code: null,
    severity: 'block',
    owner: 'agent',
    title: 'iii-sdk 0.24.4 is older than 0.26',
    detail: 'The deploy would go green and the http group would serve no route.',
    file: null,
    line: null,
    excerpt: null,
    fix: 'Upgrade iii-sdk to 0.27.0 in the lockfile.',
  },
  {
    id: 'p4',
    source: 'check_here',
    code: 'BUN_NO_LOCK',
    cloud_code: null,
    severity: 'block',
    owner: 'person',
    title: 'Runs with bun, but the repository has only a pnpm lock',
    detail: 'The cloud installs bun projects from bun.lock: nothing would be installed.',
    file: null,
    line: null,
    excerpt: null,
    fix: 'Run with node and keep the pnpm lock, or add a bun.lock.',
  },
  {
    id: 'p5',
    source: 'check_here',
    code: 'MIGRATE_IN_CD',
    cloud_code: null,
    severity: 'warn',
    owner: 'agent',
    title: 'Migrations run in your CD, not in the cloud',
    detail: '.github/workflows/cd.yml runs db:migrate; it moves to cloud.release.',
    file: null,
    line: null,
    excerpt: null,
    fix: 'Move the db:migrate command to cloud.release in the compose file.',
  },
  {
    id: 'p6',
    source: 'check_here',
    code: 'NO_HEALTHCHECK',
    cloud_code: null,
    severity: 'warn',
    owner: 'agent',
    title: 'No healthcheck',
    detail: 'Without one, a deploy is Ready even when nothing answers.',
    file: null,
    line: null,
    excerpt: null,
    fix: 'Add cloud.healthcheck with a path that answers 200.',
  },
]

export const stages = (s: 'passed' | 'none'): Run['stages'] => [
  {
    name: 'install',
    status: s === 'passed' ? 'passed' : 'pending',
    detail: 'pnpm install --frozen-lockfile',
    duration_ms: 21_000,
  },
  { name: 'build', status: 'passed', detail: 'cloud.build', duration_ms: 9_000 },
  { name: 'release', status: 'passed', detail: 'db:migrate', duration_ms: 4_000 },
  { name: 'compose_up', status: 'passed', detail: 'registry, http, storage', duration_ms: 12_000 },
  { name: 'workers', status: 'passed', detail: null, duration_ms: 3_000 },
  { name: 'routes', status: 'passed', detail: null, duration_ms: 1_000 },
  { name: 'healthcheck', status: 'passed', detail: null, duration_ms: 1_000 },
]

export const run: Run = {
  id: 'run1',
  check_id: 'chk-ready',
  commit: COMMIT,
  engine: '0.27.0',
  status: 'passed',
  started_at: T(3),
  finished_at: new Date(Date.parse(T(3)) + 58_000).toISOString(),
  stages: stages('passed'),
  workers: { ready: 3, total: 3 },
  routes: { count: 14, prefix: '/api' },
  healthcheck: { path: '/api/health', status: 200 },
  not_covered: ["The cloud's build container", "Production's secret values", 'Its machine size', 'Real traffic'],
  log_tail: [
    '[http]     14 routes under /api',
    '[registry] ready, catalog loaded',
    '[engine]   3 of 3 workers registered',
    'GET /api/health 200 4ms',
  ],
  error: null,
}

export const confirmations: Check['plan']['confirmations'] = {
  resources: [{ kind: 'postgres', alias: 'db', binding: 'DATABASE_URL', tier: 'shared' }],
  overrides: [
    { setting: 'Web', console: true, file: false },
    { setting: 'healthcheck', console: null, file: '/api/health' },
    { setting: 'release command', console: null, file: 'db:migrate' },
  ],
}

export const check = (ready: boolean, withRun: boolean): Check => ({
  id: ready ? 'chk-ready' : 'chk-blocked',
  environment_id: ENV.id,
  commit: COMMIT,
  base: 'a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9',
  checked_at: T(2),
  duration_ms: 2100,
  plan: {
    status: ready ? 'clean' : 'blocked',
    fingerprint: FINGERPRINT,
    digest: 'd1',
    blocking: ready ? 0 : 3,
    warnings: 0,
    at_least: !ready,
    confirmations: ready ? confirmations : { resources: [], overrides: [] },
    web_candidates: null,
    error: null,
  },
  here: { problems: ready ? 0 : 4, iii_version: '0.27.0', accepts_cloud_block: true },
  problems: ready ? [] : problems,
  run: withRun ? run : null,
  ready,
  report: ready ? '## iii cloud readiness\nCloud plan for 3c4d2e0: no blocking issue.' : '',
})

export const request = (
  name: string,
  mode: SecretRequest['mode'],
  state: SecretRequest['state'],
  description: string,
  min = 1,
): SecretRequest => ({
  id: `sr-${name}`,
  environment_id: ENV.id,
  name,
  description,
  mode,
  state,
  cloud: null,
  local: null,
  asked_at: T(min),
  resolved_at: state === 'waiting' ? null : T(0),
  error: null,
  external: false,
})

export const pr: PullRequest = {
  number: 12,
  url: 'https://github.com/iii-hq/registry/pull/12',
  title: 'Make registry ready for iii cloud',
  state: 'OPEN',
  head: 'iii-cloud/ready-production',
  head_sha: COMMIT,
  base: 'main',
  body: [
    '## iii cloud readiness',
    '',
    'Cloud plan for 3c4d2e0: no blocking issue.',
    '',
    'Ran here like the cloud: 3 of 3 workers, 14 routes under `/api`, healthcheck 200.',
    '',
    'Changes: compose lock, iii-sdk 0.27.0, `cloud.release`, `cloud.web`, `cloud.healthcheck`, the http and storage folders.',
    '',
    "Not covered: the cloud's build container, production's secret values, its machine size and real traffic.",
  ].join('\n'),
  checks: 'pending',
  mergeable: 'MERGEABLE',
  main_moved: false,
}

export const DEPLOY_BASE: Deploy = {
  id: 'dep-7a1f3b2',
  number: 41,
  environment_id: ENV.id,
  commit: { sha: '7a1f3b2c9d8e0f1a2b3c4d5e6f7a8b9c0d1e2f3a', message: 'Make registry ready for iii cloud (#12)' },
  type: 'push',
  status: 'in_progress',
  status_detail: null,
  stages: [],
  release_row: null,
  outcome: 'running',
  error: null,
  log_tail: [],
  pr: 12,
  replaced_by: null,
  approved_by_worker: true,
  checked_ready: true,
  created_at: T(4),
  finished_at: null,
  console_url: 'https://cloud.iii.dev/iii/registry/production/deployments/41',
}

export const st = (name: string, status: string, detail: string | null, ms: number | null) => ({
  name,
  status,
  detail,
  started_at: ms == null ? null : T(10),
  finished_at: ms == null ? null : new Date(Date.parse(T(10)) + ms).toISOString(),
})

export const following: Deploy = {
  ...DEPLOY_BASE,
  stages: [
    st('validation', 'passed', 'the plan matches the one you approved', 5_000),
    st('build', 'passed', 'registry, http, storage', 158_000),
    st('resources', 'passed', 'shared Postgres created', 41_000),
    st('config_secrets', 'passed', null, 4_000),
    { ...st('workers', 'in_progress', '2 of 3 ready', null), started_at: T(1) },
    st('routes_tls', 'pending', 'routes under /api and GET /api/health 200', null),
  ],
  release_row: { status: 'passed', duration_ms: 22_000 },
}

export const failedDeploy: Deploy = {
  ...DEPLOY_BASE,
  id: 'dep-5f81a09',
  number: 42,
  commit: { sha: '5f81a09b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f', message: null },
  status: 'failed',
  outcome: 'failed',
  stages: [
    st('validation', 'passed', null, 6_000),
    st('build', 'passed', null, 161_000),
    st('resources', 'passed', 'reused', 0),
    st('config_secrets', 'passed', null, 4_000),
    st('workers', 'failed', 'registry · OOMKilled', 72_000),
    st('routes_tls', 'pending', null, null),
  ],
  release_row: null,
  error: {
    stage: 'workers',
    worker: 'registry',
    code: 'OOMKilled',
    hint: "This is what the run here can't cover: production's machine size. Your agent has the stage, the code and the last log lines.",
    action: null,
  },
  log_tail: [
    '[registry] loading the catalog into memory',
    '[registry] killed: out of memory',
    '[engine]   worker registry exited (137)',
    '[engine]   restarting registry (2 of 3)',
    '[registry] killed: out of memory',
  ],
  created_at: T(8),
  finished_at: T(3),
  console_url: 'https://cloud.iii.dev/iii/registry/production/deployments/42',
}

export const earlierReady: Deploy = {
  ...DEPLOY_BASE,
  id: 'dep-old',
  number: 40,
  outcome: 'ready',
  status: 'ready',
  commit: { sha: '1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b', message: null },
  created_at: T(60 * 24),
  finished_at: T(60 * 24 - 8),
  pr: null,
  stages: [],
}

export const base: Status = {
  worker: { version: '0.1.0', iii_version: '0.27.0', cloud_engine: '0.27.0' },
  connection: connected,
  project: project(true),
  check: check(false, false),
  secrets: [],
  variables: [],
  pr: null,
  pr_error: null,
  deploys: [],
  refreshed_at: T(0),
}

export const variables: Status['variables'] = [
  {
    name: 'DATABASE_URL',
    type: 'builtin',
    status: 'set',
    required_by: ['Bound by the Postgres resource; a local Postgres for the run here'],
  },
  {
    name: 'SESSION_SECRET',
    type: 'secret',
    status: 'empty_default',
    required_by: ['Empty by default in the compose file'],
  },
  {
    name: 'SUPABASE_URL',
    type: 'variable',
    status: 'empty_default',
    required_by: ["Empty by default: the dashboard's sign-in stays off"],
  },
  {
    name: 'SUPABASE_ANON_KEY',
    type: 'secret',
    status: 'empty_default',
    required_by: ["Empty by default: the dashboard's sign-in stays off"],
  },
  {
    name: 'OPENAI_API_KEY',
    type: 'secret',
    status: 'empty_default',
    required_by: ['Only used when EMBEDDING_PROVIDER is openai'],
  },
]

export const secretRequests = [
  request(
    'SESSION_SECRET',
    'ask',
    'waiting',
    "Signs the organization dashboard's sessions. Left empty, sign-in stays off, which is fine while you only validate.",
  ),
  request('SUPABASE_URL', 'decide', 'noted', "Empty by default: the dashboard's sign-in stays off", 5),
  request('SUPABASE_ANON_KEY', 'decide', 'noted', "Empty by default: the dashboard's sign-in stays off", 5),
  request('OPENAI_API_KEY', 'not_needed', 'noted', 'Only used when EMBEDDING_PROVIDER is openai', 5),
]

export const agent = (state: AgentRun['state']): AgentRun => ({
  handoff_id: 'ho-1',
  session_id: 'sess-1',
  state,
  started_at: T(17),
  last_activity_at: state === 'stalled' ? T(4) : new Date(NOW - 12_000).toISOString(),
  steps: 14,
  current: state === 'done' ? null : 'Upgrading iii-sdk to 0.27.0 in api/package.json',
  working_on: state === 'working' ? ['p3'] : [],
  note: state === 'working' ? 'Bumping the sdk and regenerating the lock' : null,
  question:
    state === 'waiting_for_you' ? 'Keep path:// for http and storage, or download the binaries in the build?' : null,
})

// Two of the six problems are already gone from the newest check.
export const agentStatus: Status = {
  ...base,
  check: {
    ...check(false, false),
    problems: problems.slice(2),
    here: { problems: 2, iii_version: '0.27.0', accepts_cloud_block: true },
  },
  // The six problems the hand-off was made for: two are gone from the newest check.
  handoff_brief: {
    handoff_id: 'ho-1',
    kind: 'ready',
    repository: 'iii-hq/registry',
    project: 'registry · production',
    worktree: { path: '~/workspaces/registry-iii-cloud', branch: 'iii-cloud/ready-production' },
    checkout_path: '/home/you/workspaces/registry',
    check_id: 'chk-blocked',
    commit: COMMIT,
    agent_problems: problems
      .filter((p) => p.owner === 'agent')
      .map((p) => ({ title: p.title, file: p.file, line: p.line, detail: p.detail, fix: p.fix })),
    person_problems: problems
      .filter((p) => p.owner === 'person')
      .map((p) => ({ title: p.title, file: p.file, line: p.line, detail: p.detail, fix: p.fix })),
    deploy: null,
    next_step: '',
  },
}

export const ready = check(true, true)

/** The full cloud path of an environment route, for `Board.cloud` keys and payload checks. */
export const apiPath = (path: string, env = ENV.id) => `/teams/${TEAM.id}/environments/${env}${path}`

/** Records the chat cards of board 11 start from (`message.output.details`). */
export const cardRecords = {
  connect: connected,
  checkBlocked: check(false, false),
  secret: secretRequests[0],
  checkReady: ready,
  watch: following,
  failed: failedDeploy,
  waiting,
  signedOut,
}
