// First-time boards: setup-signin (the code), setup-new (a repository iii cloud has not imported: a new project)
// and setup-existing (the repository's project with two environments). Nothing past the confirmation lives
// here: values, the check and the deploy are the Deploy place's boards.

import type { CloudProjectChoice, Project } from '../types'
import { apiPath, type Board, base, COMMIT, ENV, project, TEAM, waiting } from './base'

export const DEV = { id: 'e-dev', name: 'dev', production: false, branch: 'develop' }
export const FILE = 'api/worker-compose-prod.yaml'

/** A repository iii cloud has not imported yet; it lists the two compose files of GitHub's `main`. */
export const notImported: Project = {
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

/** The repository's project, two environments, and three projects of other repositories. */
export const chosen: Project = {
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

/** The compose files of the worktree: the environment's, another that can be picked, one iii cloud cannot take. */
export const files = [
  { path: FILE, choosable: true, suggested_environment: null, issue: null, workers: 11, has_cloud_block: true },
  { path: 'api/worker-compose.staging.yaml', choosable: true, suggested_environment: null, issue: null, workers: 11 },
  { path: 'api/workers-compose.yaml', choosable: false, suggested_environment: null, issue: 'NOT_WORKER_COMPOSE' },
]

export type SetupBoard = 'setup-signin' | 'setup-new' | 'setup-existing'

export const boards: Record<SetupBoard, Board> = {
  'setup-signin': { status: { ...base, connection: waiting, project: null, check: null }, context: { at: 'setup' } },
  'setup-new': { status: { ...base, project: notImported, check: null }, context: { at: 'setup', step: 'confirm' } },
  'setup-existing': {
    status: { ...base, project: chosen, check: null, compose_files: files },
    context: { at: 'setup', step: 'confirm' },
    cloud: {
      [apiPath('/deployments', DEV.id)]: {
        deployments: [
          {
            id: 'cd-6',
            number: 6,
            status: 'ready',
            current: true,
            commit: { sha: '9c04e7f1a2b3c4d5e6f708192a3b4c5d6e7f8091' },
          },
        ],
      },
    },
  },
}
