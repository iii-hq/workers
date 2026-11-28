// The environment page's boards: manage-overview (dev with a passed check: the home's data) and env-page (Env1:
// dev serves #6, the folder has one commit it doesn't serve, #2 failed earlier).

import type { Status } from '../types'
import { apiPath, type Board, base } from './base'
import { DEV as DEV_DEPLOYS, devDeploys, devList, devProject } from './deploys'
import { DEV, health, home, homeCloud, NEW_SHA } from './manage'

const manageOverview: Board = {
  status: home,
  context: { at: 'env', env: DEV.id },
  cloud: homeCloud,
}

/** dev on its own branch: serving #6, the folder on feat/search-rank one commit ahead, nothing uncommitted. */
export const envPageStatus: Status = {
  ...base,
  project: {
    ...devProject,
    checkout: {
      branch: 'feat/search-rank',
      head: NEW_SHA,
      changed_files: [],
      ahead: 1,
      not_served: [{ sha: NEW_SHA, message: 'Search: tweak weights' }],
    },
  },
  check: null,
  deploys: devDeploys,
}

const envPage: Board = {
  status: envPageStatus,
  context: { at: 'env', env: DEV_DEPLOYS.id },
  cloud: {
    [apiPath('/deployments')]: { deployments: [], next_cursor: null },
    [apiPath('/deployments', DEV_DEPLOYS.id)]: devList,
    [apiPath('/health', DEV_DEPLOYS.id)]: health,
  },
}

// Keyed by literal ids so `board('manage-overview')` type-checks.
export const boards = { 'manage-overview': manageOverview, 'env-page': envPage }
