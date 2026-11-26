// Deploys tab boards (U7): manage-deploys-failed (Mng2; Mng10 at 390 wide as manage-deploys-phone) and
// manage-rollback (Mng7). dev deploys its own branch; #6 serves, #5 served before it, #2 failed while
// nothing was live yet.

import type { Deploy, Status } from '../types'
import { apiPath, type Board, base, DEPLOY_BASE, ENV, project, st, T } from './base'

export const DEV = { id: '4d6e8f0a-1b2c-4d3e-9f5a-6b7c8d9e0f1a', name: 'dev', production: false, branch: 'dev' }

const here = project(true)
export const devProject: NonNullable<Status['project']> = {
  ...here,
  default_branch: 'main',
  environment: { ...DEV, manifest: 'api/worker-compose-prod.yaml', url: 'https://registry-2-dev.iii.cloud' },
  environments: [ENV, DEV],
  workspace: { mode: 'worktree', path: '~/workspaces/registry-iii-cloud', branch: 'iii-cloud/ready-dev' },
}

const passed = () => [
  st('validation', 'passed', null, 5_000),
  st('build', 'passed', null, 92_000),
  st('resources', 'passed', null, 1_000),
  st('config_secrets', 'passed', null, 3_000),
  st('workers', 'passed', '9 of 9 ready', 24_000),
  st('routes_tls', 'passed', null, 4_000),
]

const dep = (n: number, sha: string, message: string, min: number, extra: Partial<Deploy> = {}): Deploy => ({
  ...DEPLOY_BASE,
  id: `dep-${n}`,
  number: n,
  environment_id: DEV.id,
  commit: { sha, message },
  type: 'push',
  pr: null,
  status: 'ready',
  outcome: 'ready',
  stages: passed(),
  approved_by_worker: true,
  checked_ready: true,
  created_at: T(min),
  finished_at: T(min - 3),
  console_url: `https://cloud.iii.dev/iii/registry/dev/deployments/${n}`,
  ...extra,
})

const failedAt = (n: number, min: number): Deploy =>
  dep(n, '86c49e1d2a3b4c5d6e7f8091a2b3c4d5e6f70812', 'Ready the registry', min, {
    status: 'failed',
    outcome: 'failed',
    checked_ready: false,
    approved_by_worker: false,
    stages: [
      st('validation', 'passed', null, 5_000),
      st('build', 'passed', null, 88_000),
      st('resources', 'passed', 'shared Postgres created', 41_000),
      st('config_secrets', 'passed', null, 3_000),
      st('workers', 'failed', 'http · ROUTE_OUTSIDE_HTTP_GROUP', 30_000),
      st('routes_tls', 'pending', null, null),
    ],
    error: { stage: 'workers', worker: 'http', code: 'ROUTE_OUTSIDE_HTTP_GROUP', hint: '', action: null },
    log_tail: ['group app has no http package: routes are not served'],
    failure: {
      code: 'ROUTE_OUTSIDE_HTTP_GROUP',
      stage: 'workers',
      worker: 'http',
      title: 'Your app starts its own web server; iii cloud only sends traffic to its http package',
      hint: null,
      log_tail: [
        'registered 41 routes in group app',
        'group app has no http package: routes are not served',
        'code ROUTE_OUTSIDE_HTTP_GROUP',
      ],
    },
  })

export const devDeploys: Deploy[] = [
  dep(6, '9c04e7f1a2b3c4d5e6f708192a3b4c5d6e7f8091', 'Search: rank by stars', 6, { check_id: 'chk-6c' }),
  dep(5, 'd738d2f0a1b2c3d4e5f60718293a4b5c6d7e8f90', 'Drop the separate web container', 742, { check_id: 'chk-5b' }),
  dep(4, 'd738d2f0a1b2c3d4e5f60718293a4b5c6d7e8f90', 'Drop the separate web container', 755, {
    status: 'replaced',
    outcome: 'replaced',
    replaced_by: 'dep-5',
    stages: [st('validation', 'passed', null, 5_000)],
    approved_by_worker: false,
    finished_at: T(742),
  }),
  dep(3, '58e516a9b8c7d6e5f4a3b2c1d0e9f8a7b6c5d4e3', 'Route through the http package', 762, { check_id: 'chk-3d' }),
  failedAt(2, 773),
  failedAt(1, 800),
]

/** iii cloud's view of the same deployments (`GET /deployments`): who started them and which one serves. */
export const devList = {
  deployments: devDeploys.map((d) => ({
    id: d.id,
    number: d.number,
    environment_id: DEV.id,
    type: d.type,
    status: d.status,
    status_detail: null,
    current: d.number === 6,
    commit: { sha: d.commit.sha, branch: 'dev', message: d.commit.message },
    stages: [],
    error: null,
    rollback_of: null,
    replaced_by: d.replaced_by,
    triggered_by: { user_id: d.number >= 5 ? 'u1' : 'u2', api_key_id: null },
    created_at: d.created_at,
    started_at: d.created_at,
    finished_at: d.finished_at,
  })),
  next_cursor: null,
}

const devStatus: Status = { ...base, project: devProject, check: null, deploys: devDeploys }

/** `GET /logs` for #2's workers, newest first as iii cloud answers. */
const workerLog = {
  lines: [
    'code ROUTE_OUTSIDE_HTTP_GROUP',
    'first unserved route: DELETE /orgs/:handle/invites/:id',
    'group app has no http package: routes are not served',
    'registered 41 routes in group app',
    'worker http (from your code: ./workers/http) started in group app',
  ].map((message, i) => ({
    time: T(773 - i / 60),
    source: 'worker',
    deployment_id: 'dep-2',
    group: 'app',
    worker: 'http',
    level: i < 3 ? 'error' : 'info',
    message,
    trace_id: null,
    span_id: null,
  })),
  next_cursor: null,
}

const failedCloud = {
  [apiPath('/deployments', DEV.id)]: devList,
  [apiPath('/logs?deployment_id=dep-2&source=worker', DEV.id)]: workerLog,
  [apiPath('/logs?deployment_id=dep-2&source=release', DEV.id)]: { lines: [], next_cursor: null },
}

const manageDeploysFailed: Board = {
  status: devStatus,
  context: { at: 'env', env: DEV.id, deploy: 'dep-2' },
  cloud: failedCloud,
}

const manageRollback: Board = {
  status: devStatus,
  context: { at: 'env', env: DEV.id, deploy: 'dep-5' },
  dialog: true,
  cloud: {
    [apiPath('/deployments', DEV.id)]: devList,
    [apiPath('/deployments/dep-5/rollback', DEV.id)]: {
      target: { id: 'dep-5', number: 5, commit: 'd738d2f' },
      returns: ['code', 'worker images', 'config version 4', 'the workers of each machine group'],
      stays: ['data', 'secrets', 'resources created later', 'group sizes and replicas', "the environment's settings"],
      side_effects: ['Migrations run by #6 are not reverted: 0008_trgm.sql stays.'],
      queue: 'Nothing is queued.',
    },
  },
}

// Keyed by literal ids so `board('manage-rollback')` type-checks.
export const boards = {
  'manage-deploys-failed': manageDeploysFailed,
  'manage-deploys-phone': { ...manageDeploysFailed, width: 390 },
  'manage-rollback': manageRollback,
}
