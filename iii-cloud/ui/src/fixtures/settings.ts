// Secrets and variables boards (Env4): manage-settings. dev reads six names; STRIPE_SECRET_KEY changed 2 minutes
// after the serving deployment started, so the restart notice shows.

import type { Status, Values } from '../types'
import { apiPath, type Board, base, check, DEPLOY_BASE, ENV, project, T } from './base'
import { valueItem } from './values'

/** The board's `dev`: an environment on its own branch, beside production. */
export const DEV = { id: '2f6e1d0c-8b7a-4c5d-9e3f-1a2b3c4d5e6f', name: 'dev', production: false, branch: 'dev' }

const ready = check(true, false)

export const settingsStatus: Status = {
  ...base,
  project: {
    ...project(true),
    default_branch: 'main',
    environment: { ...DEV, manifest: 'api/worker-compose-prod.yaml', url: 'https://registry-2-dev.iii.cloud' },
    environments: [ENV, DEV],
    workspace: { mode: 'worktree', path: '~/workspaces/registry-iii-cloud', branch: 'iii-cloud/ready-dev' },
  },
  check: { ...ready, environment_id: DEV.id },
  // What dev serves: started 30 minutes ago, before STRIPE_SECRET_KEY changed.
  deploys: [
    {
      ...DEPLOY_BASE,
      id: 'dep-dev-9c04e7f',
      number: 18,
      environment_id: DEV.id,
      commit: { sha: '9c04e7f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7', message: 'Rank search by stars' },
      status: 'ready',
      outcome: 'ready',
      pr: null,
      created_at: T(30),
      finished_at: T(25),
    },
  ],
}

const variable = (
  name: string,
  type: 'secret' | 'variable',
  status: 'set' | 'empty_default',
  applies: string | null,
  min: number | null,
  more: { value?: string; required_by?: string[] } = {},
) => ({
  name,
  type,
  value: more.value ?? null,
  status,
  required_by: more.required_by ?? [],
  applies,
  updated_at: min === null ? null : T(min),
})

/** `GET …/variables`: what iii cloud has for dev, and when each one reaches the machines. */
export const devVariables = {
  variables: [
    variable('STRIPE_SECRET_KEY', 'secret', 'set', 'after_restart:app,jobs', 2, { required_by: ['billing'] }),
    variable('GITHUB_CLIENT_SECRET', 'secret', 'set', 'live', 60 * 13),
    variable('GITHUB_CLIENT_ID', 'variable', 'set', 'live', 60 * 13, { value: 'Iv1.8a61f9b3a7aba766' }),
    variable('SEARCH_WEIGHT_STARS', 'variable', 'set', null, 28, { value: '0.35' }),
    variable('ASSET_BUCKET_KEY', 'secret', 'empty_default', null, null),
    // Set in iii cloud's console; no compose file reads it.
    variable('LEGACY_FLAG', 'variable', 'set', 'live', 60 * 24, { value: 'on' }),
  ],
}

const item = (name: string, over: Parameters<typeof valueItem>[1]) =>
  valueItem(name, { containers: ['search'], ...over })

/** `iii-cloud::values`: the six names api/worker-compose-prod.yaml reads in dev. */
export const devSecretValues: Values = {
  environment_id: DEV.id,
  environment: DEV.name,
  manifest: 'api/worker-compose-prod.yaml',
  read_from: 'workspace',
  cloud: 'read',
  save: 'ok',
  items: [
    item('STRIPE_SECRET_KEY', { need: 'required', default: null, in_cloud: 'set', containers: ['billing'] }),
    item('GITHUB_CLIENT_SECRET', { need: 'required', default: null, in_cloud: 'set' }),
    item('GITHUB_CLIENT_ID', { type: 'variable', in_cloud: 'set', value: 'Iv1.8a61f9b3a7aba766' }),
    item('SEARCH_WEIGHT_STARS', { type: 'variable', in_cloud: 'set', value: '0.35' }),
    item('DATABASE_PRIMARY_URL', { need: 'required', default: null, database: true, containers: ['api'] }),
    item('ASSET_BUCKET_KEY', {}),
  ],
}

const manageSettings: Board = {
  status: settingsStatus,
  context: { at: 'secrets', env: DEV.id },
  values: devSecretValues,
  cloud: { [apiPath('/variables', DEV.id)]: devVariables },
}

// Literal keys, like the other area modules: `board('manage-settings')` type-checks.
export const boards = { 'manage-settings': manageSettings }
