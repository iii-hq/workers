// New environment boards (U10): manage-new-env.

import { apiPath, type Board, base, ENV, earlierReady, project, TEAM } from './base'

/** The project's second environment: not production, so a trial copies it by default. */
export const DEV = { id: '6c2d9e1f-3a4b-4c5d-8e6f-7a8b9c0d1e2f', name: 'dev', production: false, branch: 'develop' }

const here = project(true)

const group = (name: string, size: string) => ({
  name,
  workers: [name, 'database', 'queue', 'state'],
  current_workers: [name, 'database', 'queue', 'state'],
  size,
  effective_size: size,
  operation: null,
  replicas: { min: 1, max: 1, current: 1 },
  scale: null,
  fixed_reason: null,
  pending: false,
})

const database = (id: string, alias = 'main', status = 'ready') => ({
  id,
  kind: 'database',
  alias,
  mode: 'managed',
  status,
  used_by: ['database', 'api'],
  size: 'small',
  tier: 'shared',
  backup_retention_days: 7,
  cors_origins: null,
  connection: [],
  credentials: { rotated_at: null, rotating: false },
  snapshot: null,
  delete_after: null,
  created_at: '2026-10-10T12:00:00Z',
})

const machines = (groups: ReturnType<typeof group>[]) => ({
  status: 'running',
  current_deployment_id: null,
  engine: { status: 'ready', version: '0.27.0', restarted_at: null },
  groups,
  limits: { groups: 3, replicas_per_group: 1 },
})

// Mng8: from the sidebar's "+ New environment", on a feature branch; dev and production to copy from.
const manageNewEnv: Board = {
  status: {
    ...base,
    deploys: [earlierReady],
    project: {
      ...here,
      checkout: { ...here.checkout, branch: 'feat/search-rank' },
      default_branch: 'main',
      environments: [ENV, DEV],
      cloud_projects: [{ id: 'p1', name: 'registry', slug: 'registry', team: TEAM, environments: [ENV, DEV] }],
    },
  },
  context: { at: 'new-env', branch: 'feat/search-rank' },
  cloud: {
    [apiPath('/machines', DEV.id)]: machines([group('app', '0.25x'), group('http', '0.25x'), group('jobs', '0.25x')]),
    [apiPath('/resources', DEV.id)]: { resources: [database('res-dev')] },
    [apiPath('/machines', ENV.id)]: machines([group('app', '1x'), group('jobs', '2x')]),
    [apiPath('/resources', ENV.id)]: {
      resources: [database('res-prod'), database('res-old', 'old', 'deleted')],
    },
  },
}

// Keyed by literal ids so `board('manage-new-env')` type-checks.
export const boards = { 'manage-new-env': manageNewEnv }
