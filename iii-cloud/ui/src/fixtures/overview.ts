// Overview tab boards (U6): manage-overview, dev's Overview with the home's data and no domain yet.

import { apiPath, type Board } from './base'
import { DEV, home, homeCloud } from './manage'

const manageOverview: Board = {
  status: home,
  context: { at: 'env', env: DEV.id },
  cloud: { ...homeCloud, [apiPath('/domains', DEV.id)]: { domains: [] } },
}

// Keyed by literal ids so `board('manage-overview')` type-checks.
export const boards = { 'manage-overview': manageOverview }
