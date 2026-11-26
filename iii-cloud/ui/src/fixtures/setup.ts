// Setup boards (U3): setup-signin, setup-code, setup-target, setup-ship, setup-deploying, setup-live.
// One story: registry is set up for `dev`, a direct environment (it follows `develop`, not `main`).

import type { CloudProjectChoice, Deploy, Project, Status } from '../types'
import {
  agentStatus,
  apiPath,
  type Board,
  base,
  COMMIT,
  DEPLOY_BASE,
  ENV,
  failedDeploy,
  following,
  project,
  ready,
  T,
  TEAM,
  waiting,
} from './base'

export const DEV = { id: 'e-dev', name: 'dev', production: false, branch: 'develop' }
export const FILE = 'api/worker-compose-prod.yaml'

/** Onb2: a repository iii cloud has not imported yet; it lists the two compose files of GitHub's `main`. */
const notImported: Project = {
  ...project(false),
  cloud_project: null,
  environment: null,
  environments: [],
  default_branch: 'main',
  checkout: {
    branch: 'main',
    head: COMMIT,
    changed_files: ['api/src/index.ts', 'api/src/routes.ts', 'api/worker-compose-prod.yaml', 'README.md'],
    ahead: 0,
  },
  importable: {
    team_id: TEAM.id,
    repository_id: 1,
    manifests: [
      {
        path: FILE,
        choosable: true,
        suggested_environment: 'production',
        issue: null,
        workers: 9,
        has_cloud_block: true,
      },
      {
        path: 'api/worker-compose.yaml',
        choosable: true,
        suggested_environment: 'development',
        issue: null,
        workers: 9,
        has_cloud_block: false,
      },
    ],
  },
}

export const registry: CloudProjectChoice = {
  id: 'p1',
  name: 'registry',
  slug: 'registry',
  team: { id: TEAM.id, name: TEAM.name },
  environments: [
    { ...ENV, manifest: FILE },
    { ...DEV, manifest: FILE },
  ],
}

/** Onb3: the repository's project, two environments, and three projects of other repositories. */
const chosen: Project = {
  ...project(false),
  default_branch: 'main',
  cloud_projects: [registry],
  environments: [ENV, DEV],
  other_cloud_projects: [
    { id: 'x1', name: 'hello-iii', repository: 'iii-hq/hello-iii' },
    { id: 'x2', name: 'docs-site', repository: 'iii-hq/docs-site' },
    { id: 'x3', name: 'learn-iii', repository: 'iii-hq/learn-iii' },
  ],
}

/** Onb6 on: `dev` is selected and its check passed after the agent's two commits. */
const devProject: Project = {
  ...project(true),
  default_branch: 'main',
  cloud_projects: [registry],
  environments: [ENV, DEV],
  environment: { ...DEV, manifest: FILE, url: 'https://registry-dev.iii.cloud' },
}

const passed = {
  ...ready,
  environment_id: DEV.id,
  commits: [
    { sha: COMMIT, message: 'Pin every worker version' },
    { sha: 'b0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9', message: 'Route web traffic through the http package' },
  ],
  plan: { ...ready.plan, summary: { workers: 9, groups: ['web', 'jobs', 'storage'] } },
}

const variable = (name: string, type: 'secret' | 'variable' | 'builtin', status: 'set' | 'empty_default') => ({
  name,
  type,
  status,
  required_by: [],
})

export const shipStatus: Status = {
  ...base,
  project: devProject,
  check: passed,
  variables: [
    variable('DATABASE_URL', 'builtin', 'set'),
    variable('SESSION_SECRET', 'secret', 'set'),
    variable('OPENAI_API_KEY', 'secret', 'set'),
    variable('SENTRY_DSN', 'variable', 'set'),
    variable('SUPABASE_URL', 'variable', 'empty_default'),
  ],
  // The agent's hand-off: its problems are gone from the passed check.
  handoff_brief: agentStatus.handoff_brief ? { ...agentStatus.handoff_brief, pushes_directly: true } : null,
}

export const deploying: Deploy = {
  ...following,
  id: 'dep-5',
  number: 5,
  environment_id: DEV.id,
  commit: { sha: COMMIT, message: 'Pin every worker version' },
  pr: null,
  log_tail: [],
}

const shipped: Deploy = {
  ...DEPLOY_BASE,
  id: 'dep-5',
  number: 5,
  environment_id: DEV.id,
  commit: { sha: COMMIT, message: 'Pin every worker version' },
  status: 'ready',
  outcome: 'ready',
  pr: null,
  created_at: T(12),
  finished_at: T(6),
}

const failedAt = (n: number, min: number): Deploy => ({
  ...failedDeploy,
  id: `dep-${n}`,
  number: n,
  environment_id: DEV.id,
  created_at: T(min),
  finished_at: T(min - 5),
})

const summary = (id: string, min: number, isReady: boolean) => ({
  id,
  environment_id: DEV.id,
  commit: COMMIT,
  checked_at: T(min),
  duration_ms: 2100,
  ready: isReady,
  problems: isReady ? 0 : 6,
  handoff_id: isReady ? null : 'ho-1',
  manifest: null,
})

export type SetupBoard =
  | 'setup-signin'
  | 'setup-code'
  | 'setup-target'
  | 'setup-ship'
  | 'setup-deploying'
  | 'setup-live'

export const boards: Record<SetupBoard, Board> = {
  'setup-signin': { status: { ...base, connection: waiting, project: null, check: null }, context: { at: 'setup' } },
  'setup-code': { status: { ...base, project: notImported, check: null }, context: { at: 'setup', step: 'confirm' } },
  'setup-target': {
    status: { ...base, project: chosen, check: null },
    context: { at: 'setup' },
    cloud: {
      [apiPath('/deployments')]: { deployments: [] },
      [apiPath('/deployments', DEV.id)]: { deployments: [{ id: 'cd-3', number: 3, status: 'failed', current: false }] },
    },
  },
  'setup-ship': { status: shipStatus, context: { at: 'setup' } },
  'setup-deploying': {
    status: { ...shipStatus, deploys: [deploying] },
    context: { at: 'setup' },
    cloud: {
      // Newest first, the way iii cloud answers.
      [`${apiPath('/logs', DEV.id)}?deployment_id=dep-5&source=release`]: {
        lines: [
          { message: '7 migrations applied' },
          { message: 'applying migration 0007_vector_search.sql' },
          { message: 'applying migration 0006_collections.sql' },
        ],
      },
    },
  },
  'setup-live': {
    status: {
      ...shipStatus,
      deploys: [shipped, failedAt(4, 40), failedAt(3, 60)],
      checks: [summary('chk-ready', 14, true), summary('chk-first', 86, false)],
    },
    // Live once: without a route the page would open on the project's home.
    context: { at: 'setup' },
    cloud: {
      [apiPath('/health', DEV.id)]: {
        status: 'healthy',
        checked_at: T(0),
        engine: 'ready',
        workers: Array.from({ length: 9 }, (_, i) => ({
          group: 'web',
          worker: `w${i}`,
          ready: true,
          restarts: 0,
          crashloop: false,
        })),
        events: [],
      },
    },
  },
}
