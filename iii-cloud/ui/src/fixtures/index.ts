// Every board, by id: one module per area.

import type { RunningDeploy } from '../types'
import { type Board, ENV } from './base'
import { boards as check } from './check'
import { boards as deploy, running6 } from './deploy'
import { boards as deploys } from './deploys'
import { boards as manage } from './manage'
import { boards as overview } from './overview'
import { boards as settings } from './settings'
import { boards as setup } from './setup'
import { billing, boards as start } from './start'
import { boards as values } from './values'

export type { Board } from './base'
export { cardRecords } from './base'

/** A board with dev's deploy #6 going on (DeployPill, Start's Follow deploy, the setup rail's step 6). */
const deploying = (b: Board): Board => {
  const d = { ...running6, environment_id: b.status.project?.environment?.id ?? running6.environment_id }
  return { ...b, status: { ...b.status, deploys: [d, ...b.status.deploys] } }
}

/** `running_deploys`: one of this folder's (`#n` to `env`), and billing's dev deploying #8 from its own folder. */
const here = (b: Board, n: number, env: { id: string; name: string }): RunningDeploy => {
  const p = b.status.project
  return {
    deploy: { ...running6, id: `dep-${n}`, number: n, environment_id: env.id },
    cloud_project_id: p?.cloud_project?.id ?? null,
    project_name: p?.cloud_project?.name ?? null,
    repository: p?.repository ?? null,
    environment_name: env.name,
    path: p?.path ?? null,
  }
}
export const billing8: RunningDeploy = {
  deploy: { ...running6, id: 'dep-b8', number: 8, environment_id: 'e-billing-dev' },
  cloud_project_id: billing.cloud_project_id,
  project_name: billing.cloud_project_name,
  repository: billing.repository,
  environment_name: 'dev',
  path: billing.path,
}
const moving = (b: Board, running_deploys: RunningDeploy[]): Board => ({
  ...b,
  status: { ...b.status, running_deploys },
})
const overviewB = overview['manage-overview']
const startB: Board = start.start

export const boards = {
  ...check,
  ...setup,
  ...deploy,
  ...manage,
  ...overview,
  ...deploys,
  ...settings,
  ...start,
  // Last: it adds the `values` answer to boards of other areas.
  ...values,
  'manage-overview-deploying': deploying(overview['manage-overview']),
  'setup-check-deploying': deploying(check['setup-fixing']),
  'start-deploying': deploying(start.start),
  // Every project's deploys (`running_deploys`): production #7 (not the selected env) and billing's #8.
  'manage-overview-moving': moving(overviewB, [here(overviewB, 7, ENV), billing8]),
  // The project in use deploying #6 to production, and billing's #8.
  'start-moving': moving(startB, [here(startB, 6, ENV), billing8]),
}
