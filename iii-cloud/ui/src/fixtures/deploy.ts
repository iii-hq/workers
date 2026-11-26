// Deploy from this machine boards (U4): deploy-what, deploy-review, deploy-follow-*. A feature branch
// goes to `dev`, an environment on its own branch (it deploys by a push, no pull request).

import type { CheckSummary, CommitLine, Deploy, Status } from '../types'
import { apiPath, type Board, base, DEPLOY_BASE, ENV, project, ready, request, run, st, T } from './base'

/** A full-length sha from its short form. */
const sha = (short: string) => short.padEnd(40, '0')
const line = (short: string, message: string): CommitLine => ({ sha: sha(short), message })

export const DEV = { id: '3f6b1c2d-8e9a-4b7c-9d0e-1f2a3b4c5d6e', name: 'dev', production: false, branch: 'dev' }
export const CHECKED = sha('9c04e7f')

const notServed = [
  line('a41c09e', 'Search: rank by stars'),
  line('7b2e5d1', 'Add trigram index for search'),
  line('c90f114', 'Billing worker for paid kits'),
]

export const devProject = (): NonNullable<Status['project']> => ({
  ...project(true),
  default_branch: 'main',
  checkout: {
    branch: 'feat/search-rank',
    head: notServed[0].sha,
    // `.env` is never brought: the page never offers it.
    changed_files: ['api/src/search.ts', 'api/migrations/0008_trgm.sql', '.env'],
    ahead: 3,
    not_served: notServed,
  },
  environment: { ...DEV, manifest: 'api/worker-compose-prod.yaml', url: 'https://registry-2-dev.iii.cloud' },
  environments: [ENV, DEV],
  workspace: {
    mode: 'worktree',
    path: '~/workspaces/registry-iii-cloud-dev',
    branch: 'iii-cloud/ready-dev',
    // The commit "Uncommitted changes from feat/search-rank": the agent's commits come after it.
    from_head: sha('4b8f001'),
    // The folder's HEAD when the worktree took it: nothing moved since.
    source_sha: notServed[0].sha,
  },
})

const composeFiles: Status['compose_files'] = [
  { path: 'api/worker-compose-prod.yaml', choosable: true, suggested_environment: null, issue: null, workers: 11 },
  {
    path: 'api/worker-compose.staging.yaml',
    choosable: true,
    suggested_environment: 'staging',
    issue: null,
    workers: 11,
  },
  { path: 'api/workers-compose.yaml', choosable: false, suggested_environment: null, issue: 'NOT_WORKER_COMPOSE' },
]

export const serving5: Deploy = {
  ...DEPLOY_BASE,
  id: 'dep-5',
  number: 5,
  environment_id: DEV.id,
  commit: { sha: sha('d738d2f'), message: 'Drop cloud.web' },
  type: 'push',
  status: 'ready',
  outcome: 'ready',
  pr: null,
  check_id: null,
  created_at: T(60 * 6 + 5),
  finished_at: T(60 * 6),
  console_url: 'https://cloud.iii.dev/iii/registry/dev/deployments/5',
}

/** The passed check: 6 commits dev doesn't have, 4 from the checkout and 2 by the agent. */
export const devCheck = {
  ...ready,
  id: 'chk-dev',
  environment_id: DEV.id,
  commit: CHECKED,
  base: serving5.commit.sha,
  checked_at: T(2),
  manifest: null,
  run: { ...run, check_id: 'chk-dev', commit: CHECKED },
  plan: {
    ...ready.plan,
    confirmations: { resources: [], overrides: [] },
    summary: { workers: 11, groups: ['app', 'jobs', 'web'] },
  },
  commits: [
    line('9c04e7f', 'Pin search to 0.4.2'),
    line('5e1d2a3', 'Add the billing lock'),
    line('4b8f001', 'Uncommitted changes from feat/search-rank'),
    ...notServed,
  ],
}

const summary = (id: string, min: number, ok: boolean): CheckSummary => ({
  id,
  environment_id: DEV.id,
  commit: CHECKED,
  checked_at: T(min),
  duration_ms: 2100,
  ready: ok,
  problems: ok ? 0 : 3,
  handoff_id: null,
  manifest: null,
})

/** Three checks since the last ready one (7 h ago): 11 min ago, 6 min ago, and the passing one 2 min ago. */
const checks = [
  summary('chk-dev', 2, true),
  summary('chk-2', 6, false),
  summary('chk-1', 11, false),
  summary('chk-0', 420, true),
]

const stripe = {
  ...request('STRIPE_SECRET_KEY', 'ask', 'set', 'Charges for paid kits', 8),
  environment_id: DEV.id,
  resolved_at: T(5),
}

const deploy6: Deploy = {
  ...DEPLOY_BASE,
  id: 'dep-6',
  number: 6,
  environment_id: DEV.id,
  commit: { sha: CHECKED, message: 'Pin search to 0.4.2' },
  pr: null,
  check_id: 'chk-dev',
  created_at: T(4),
  release_row: { status: 'passed', duration_ms: 6_000 },
  console_url: 'https://cloud.iii.dev/iii/registry/dev/deployments/6',
}
const head = [
  st('validation', 'passed', 'the plan matches the one you approved', 4_000),
  st('build', 'passed', 'app, jobs, web', 190_000),
  st('resources', 'passed', 'reused', 1_000),
  st('config_secrets', 'passed', null, 3_000),
]

export const running6: Deploy = {
  ...deploy6,
  stages: [
    ...head,
    { ...st('workers', 'in_progress', '8 of 11 ready', null), started_at: T(1) },
    st('routes_tls', 'pending', null, null),
  ],
}
export const live6: Deploy = {
  ...deploy6,
  status: 'ready',
  outcome: 'ready',
  finished_at: T(0),
  stages: [...head, st('workers', 'passed', '11 of 11 ready', 48_000), st('routes_tls', 'passed', null, 12_000)],
}
export const failed6: Deploy = {
  ...deploy6,
  status: 'failed',
  outcome: 'failed',
  finished_at: T(1),
  stages: [...head, st('workers', 'failed', 'search · OOMKilled', 31_000), st('routes_tls', 'pending', null, null)],
  error: {
    stage: 'workers',
    worker: 'search',
    code: 'OOMKilled',
    hint: 'Machines in the jobs group on dev have 512 MB.',
    action: null,
  },
  failure: {
    code: 'OOMKilled',
    stage: 'workers',
    worker: 'search',
    title: 'The search worker ran out of memory while starting',
    hint: "Machines in the jobs group on dev have 512 MB, and search now loads the whole trigram index at start. Your machine has more memory, so the check here couldn't see it.",
    log_tail: [
      'search: loading trigram index (1.2M rows)',
      'search: heap 498 MB / 512 MB',
      'machine jobs-1 stopped: out of memory (exit 137)',
    ],
  },
}

const dev: Status = {
  ...base,
  project: devProject(),
  check: null,
  checks,
  compose_files: composeFiles,
  deploys: [serving5],
  secrets: [stripe],
}
const reviewed: Status = { ...dev, check: devCheck }
const follow = (d: Deploy) => ({ at: 'deploy' as const, env: DEV.id, deploy: d.id })

export const boards = {
  'deploy-what': { status: dev, context: { at: 'deploy', env: DEV.id } },
  'deploy-review': { status: reviewed, context: { at: 'deploy', env: DEV.id } },
  'deploy-follow-running': {
    status: { ...reviewed, deploys: [running6, serving5] },
    context: follow(running6),
    cloud: {
      [`${apiPath('/logs', DEV.id)}?deployment_id=dep-6`]: {
        lines: [
          { worker: 'search', message: 'loading trigram index (1.2M rows)' },
          { worker: 'billing', message: 'started in group app' },
          { worker: null, message: 'applying migration 0008_trgm.sql · done' },
        ],
        next_cursor: null,
      },
    },
  },
  'deploy-follow-live': {
    status: { ...reviewed, deploys: [live6, serving5] },
    context: follow(live6),
    cloud: {
      [apiPath('/health', DEV.id)]: {
        status: 'healthy',
        checked_at: T(0),
        engine: 'ready',
        workers: Array.from({ length: 11 }, (_, i) => ({
          group: 'app',
          worker: `w${i}`,
          ready: true,
          restarts: 0,
          crashloop: false,
        })),
        events: [],
      },
    },
  },
  'deploy-follow-failed': { status: { ...reviewed, deploys: [failed6, serving5] }, context: follow(failed6) },
} satisfies Record<string, Board>
