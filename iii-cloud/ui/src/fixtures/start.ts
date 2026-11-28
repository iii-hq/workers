// Start boards (§3 of the navigation plan): the projects on this machine.
// registry is in use (production serves #40); billing is a recent project of team acme whose dev is deploying #8;
// registry-3's environment is gone (its row shows production); old was deleted in iii cloud; registry-eu is
// another cloud project of this folder whose last deploy failed.

import type { RecentProject, Status } from '../types'
import { type Board, base, connected, ENV, project, T, TEAM } from './base'
import { cloudDeploy } from './manage'

export const ACME = { id: 't2', name: 'acme', slug: 'acme' }

export const billing: RecentProject = {
  path: '/home/you/workspaces/billing',
  repository: 'iii-hq/billing',
  cloud_project_id: 'p-billing',
  cloud_project_name: 'billing',
  team_id: ACME.id,
  team_name: ACME.name,
  environment_id: 'e-billing-dev',
  environment_name: 'dev',
  mode: 'checkout',
  used_at: T(60),
}
/** An older entry: no team id, and its environment is gone from iii cloud. */
export const registry3: RecentProject = {
  path: '/home/you/workspaces/registry-3',
  repository: 'iii-hq/registry',
  cloud_project_id: 'p-r3',
  cloud_project_name: 'registry-3',
  team_name: TEAM.name,
  environment_id: 'e-r3-gone',
  environment_name: 'staging',
  mode: 'worktree',
  used_at: T(120),
}
export const old: RecentProject = {
  path: '/home/you/workspaces/old',
  repository: 'iii-hq/old',
  cloud_project_id: 'p-old',
  cloud_project_name: 'old',
  team_id: TEAM.id,
  team_name: TEAM.name,
  environment_id: 'e-old',
  mode: 'worktree',
  used_at: T(600),
}

const p = project(true)
export const startStatus: Status = {
  ...base,
  check: null,
  connection: {
    ...connected,
    user: { id: 'u1', email: 'ana@iii.dev', name: null },
    teams: [
      { ...TEAM, role: 'Developer' },
      { ...ACME, role: 'Owner' },
    ],
  },
  project: {
    ...p,
    cloud_projects: [
      { ...p.cloud_project!, team: { id: TEAM.id, name: TEAM.name } },
      { id: 'p-eu', name: 'registry-eu', slug: 'registry-eu', team: { id: TEAM.id, name: TEAM.name } },
    ],
  },
  // Newest first, with billing twice (the same folder and project is one row).
  recent_projects: [billing, { ...billing, used_at: T(900) }, registry3, old],
}

const env = (id: string, name: string, production = false) => ({ id, name, production, status: 'active' })

export const startCloud: Record<string, unknown> = {
  [`/teams/${TEAM.id}/environments/${ENV.id}/deployments`]: {
    deployments: [
      { ...cloudDeploy(40, '1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b', '', 'ready', 120, null), current: true },
    ],
  },
  [`/teams/${ACME.id}/projects/p-billing/environments`]: {
    environments: [env('e-billing-dev', 'dev'), env('e-billing-prod', 'production', true)],
  },
  [`/teams/${ACME.id}/environments/e-billing-dev/deployments`]: {
    deployments: [
      cloudDeploy(8, 'b1b2b3b4b5b6b7b8b9b0', 'Invoices', 'in_progress', 2, null),
      { ...cloudDeploy(7, 'a1b2c3d4e5f6a7b8c9d0', 'Taxes', 'ready', 120, null), current: true },
    ],
  },
  [`/teams/${TEAM.id}/projects/p-r3/environments`]: { environments: [env('e-r3-prod', 'production', true)] },
  [`/teams/${TEAM.id}/environments/e-r3-prod/deployments`]: { deployments: [] },
  // p-old answers 404: deleted, restorable until 3 Dec.
  [`/teams/${TEAM.id}/deletions`]: {
    deletions: [
      {
        kind: 'project',
        project: { id: 'p-old' },
        restorable: true,
        restorable_until: new Date(Date.UTC(2026, 11, 3, 12)).toISOString(),
      },
    ],
  },
  [`/teams/${TEAM.id}/projects/p-eu/environments`]: { environments: [env('e-eu', 'eu', true)] },
  [`/teams/${TEAM.id}/environments/e-eu/deployments`]: {
    deployments: [cloudDeploy(3, 'c1c2c3c4c5c6c7c8c9c0', 'Region', 'failed', 10, null)],
  },
}

export const boards = {
  start: { status: startStatus, context: { at: 'start' }, cloud: startCloud } satisfies Board,
  'start-empty': {
    status: { ...base, project: null, check: null, recent_projects: [] },
    context: { at: 'start' },
  } satisfies Board,
}
