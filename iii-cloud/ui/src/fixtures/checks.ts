// Checks tab board (U9): manage-checks (Mng4). dev serves 9c04e7f (#6); 2be71d0 passed since and waits.
// Earlier: three hand-offs, the newest still the agent's (it knows its kind), each deployed.

import type { CheckSummary, Status } from '../types'
import { agent, type Board, base, ready, request, T } from './base'
import { DEV, devDeploys, devProject } from './deploys'

const summary = (id: string, sha7: string, min: number, problems: number, handoff: string | null): CheckSummary => ({
  id,
  environment_id: DEV.id,
  commit: sha7.padEnd(40, '0'),
  checked_at: T(min),
  duration_ms: 2_400,
  ready: problems === 0,
  problems,
  handoff_id: handoff,
  manifest: 'api/worker-compose-prod.yaml',
})

export const devChecks: CheckSummary[] = [
  summary('chk-now', '2be71d0', 1, 0, null),
  // ho-3: the deploy of feat/search-rank, deployed as #6.
  summary('chk-6c', '9c04e7f', 12, 0, 'ho-3'),
  summary('chk-6b', '4f1a2b3', 15, 1, 'ho-3'),
  summary('chk-6a', '7e2d1c0', 20, 2, 'ho-3'),
  // ho-2: the fix of deploy #2, deployed as #5.
  summary('chk-5b', 'd738d2f', 745, 0, 'ho-2'),
  summary('chk-5a', '3a9b8c7', 750, 1, 'ho-2'),
  // ho-1: setting the project up, deployed as #3.
  summary('chk-3d', '58e516a', 765, 0, 'ho-1'),
  summary('chk-3c', '6b5a4f3', 768, 1, 'ho-1'),
  summary('chk-3b', '1c2d3e4', 770, 3, 'ho-1'),
  summary('chk-3a', '0f9e8d7', 775, 5, 'ho-1'),
]

const saved = (name: string, min: number) => ({
  ...request(name, 'ask', 'set', `${name} for dev`),
  environment_id: DEV.id,
  resolved_at: T(min),
})

const status: Status = {
  ...base,
  project: devProject,
  check: {
    ...ready,
    id: 'chk-now',
    environment_id: DEV.id,
    commit: devChecks[0].commit,
    checked_at: T(1),
    handoff_id: null,
  },
  checks: devChecks,
  deploys: devDeploys,
  secrets: [saved('GITHUB_TOKEN', 17), saved('SESSION_SECRET', 772)],
  // The agent's hand-off is still ho-3 (done): the page knows that one's kind.
  agent: { ...agent('done'), handoff_id: 'ho-3', kind: 'ready' },
}

const manageChecks: Board = { status, context: { at: 'env', env: DEV.id, tab: 'checks' } }

// Keyed by literal ids so `board('manage-checks')` type-checks.
export const boards = { 'manage-checks': manageChecks }
