// Manage boards (U5): manage-home, manage-empty, manage-stale, manage-deleted.
// A project with production (pull requests into main) and dev (its own branch), dev selected:
// dev serves 9c04e7f (#6, by you from this machine) and its check passed on 2be71d0.

import type { Deploy, Status } from '../types'
import { apiPath, type Board, base, check, connected, DEPLOY_BASE, ENV, project, T, TEAM } from './base'

export const DEV = { id: 'e-dev', name: 'dev', production: false, branch: 'develop' }
export const LIVE_SHA = '9c04e7f3b1a2c4d5e6f708192a3b4c5d6e7f8091'
export const NEW_SHA = '2be71d0c4e5f6a7b8c9d0e1f2a3b4c5d6e7f8091'
export const ME = connected.user!.id
export const ANA = 'u-ana'

/** A deployment as iii cloud lists it (`GET /deployments`). */
export const cloudDeploy = (
  n: number,
  sha: string,
  message: string,
  status: string,
  min: number,
  by: string | null,
  extra: Record<string, unknown> = {},
) => ({
  id: `dep-${n}`,
  number: n,
  environment_id: DEV.id,
  type: by ? 'manual' : 'push',
  status,
  current: false,
  commit: { sha, branch: DEV.branch, message },
  triggered_by: { user_id: by, api_key_id: null },
  created_at: T(min),
  started_at: T(min),
  finished_at: T(min - 1),
  stages: [],
  error: null,
  rollback_of: null,
  replaced_by: null,
  status_detail: null,
  ...extra,
})

export const devDeploys = [
  cloudDeploy(6, LIVE_SHA, 'Search: rank by stars', 'ready', 6, ME, { current: true }),
  cloudDeploy(5, 'd738d2f0a1b2c3d4e5f60718293a4b5c6d7e8f90', 'Drop the separate web container', 'replaced', 60, ANA),
  cloudDeploy(2, '86c49e1a2b3c4d5e6f708192a3b4c5d6e7f80912', 'Ready the registry', 'failed', 90, null),
]

const workers = [
  'registry',
  'billing',
  'search',
  'http',
  'storage',
  'cron',
  'mailer',
  'indexer',
  'thumbs',
  'webhooks',
  'postgres',
]
export const health = {
  status: 'healthy',
  checked_at: T(1),
  engine: 'ready',
  workers: workers.map((w) => ({ group: 'app', worker: w, ready: true, restarts: 0, crashloop: false })),
  events: [],
}

export const members = {
  members: [
    { user: { id: ME, email: 'you@example.com' }, role: 'developer', joined_at: T(9000) },
    { user: { id: ANA, email: 'ana@example.com', name: 'ana' }, role: 'admin', joined_at: T(9000) },
  ],
  invitations: [],
  roles: [],
  matrix: {},
}

/** The worker's own record of #6: it deployed it after a ready check. */
const local6: Deploy = {
  ...DEPLOY_BASE,
  id: 'dep-6',
  number: 6,
  environment_id: DEV.id,
  commit: { sha: LIVE_SHA, message: 'Search: rank by stars' },
  status: 'ready',
  outcome: 'ready',
  pr: null,
  approved_by_worker: true,
  checked_ready: true,
  created_at: T(6),
  finished_at: T(5),
}

const p = project(true)
export const home: Status = {
  ...base,
  connection: { ...connected, teams: [{ ...TEAM, role: 'Developer' }] },
  project: {
    ...p,
    default_branch: 'main',
    environments: [ENV, DEV],
    environment: { ...DEV, manifest: 'api/worker-compose-prod.yaml', url: 'https://registry-dev.iii.cloud' },
    workspace: { mode: 'worktree', path: '~/workspaces/registry-iii-cloud', branch: 'iii-cloud/ready-dev' },
  },
  check: {
    ...check(true, true),
    environment_id: DEV.id,
    commit: NEW_SHA,
    checked_at: T(1),
    commits: [{ sha: NEW_SHA, message: 'Search: tweak weights' }],
  },
  deploys: [local6],
}

/** What iii cloud answers for the home: production has nothing live, dev serves #6. */
export const homeCloud: Record<string, unknown> = {
  [apiPath('/deployments')]: { deployments: [], next_cursor: null },
  [apiPath('/deployments', DEV.id)]: { deployments: devDeploys, next_cursor: null },
  [apiPath('/health', DEV.id)]: health,
  [`/teams/${TEAM.id}/members`]: members,
}

const manageHome: Board = { status: home, context: { at: 'env', env: '' }, cloud: homeCloud }
const manageEmpty: Board = {
  status: { ...home, project: { ...home.project!, environments: [], environment: null }, check: null, deploys: [] },
  context: { at: 'env', env: '' },
}
// The test makes the next status read fail: the page keeps this one, from 9 minutes ago.
const manageStale: Board = {
  status: { ...home, refreshed_at: T(9) },
  context: { at: 'env', env: '' },
  cloud: homeCloud,
}
const manageDeleted: Board = {
  status: { ...home, problems: [{ code: 'PROJECT_DELETED', message: 'The project was deleted.' }] },
  context: { at: 'env', env: '' },
  cloud: {
    [`/teams/${TEAM.id}/deletions`]: {
      deletions: [
        {
          id: 'del-1',
          kind: 'project',
          name: 'registry',
          project: { id: 'p1', name: 'registry', endpoint_slug: 'registry' },
          environment_id: null,
          restorable: true,
          restorable_until: new Date(Date.UTC(2026, 11, 3, 12)).toISOString(),
          state: 'retained',
        },
      ],
    },
  },
}

// Keyed by literal ids so `board('manage-home')` type-checks.
export const boards = {
  'manage-home': manageHome,
  'manage-empty': manageEmpty,
  'manage-stale': manageStale,
  'manage-deleted': manageDeleted,
}
