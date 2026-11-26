// Settings tab boards (U8): manage-settings (Mng3, and Mng11 behind its Change… button); manage-settings-config
// (O4: the workers' config panel, two workers copied, the check still warning about ./config).

import type { Status } from '../types'
import { apiPath, type Board, base, check, DEPLOY_BASE, ENV, project, T } from './base'
import { configPreview, devConfig } from './check'

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
  check: {
    ...ready,
    environment_id: DEV.id,
    plan: {
      ...ready.plan,
      confirmations: {
        resources: [],
        overrides: [
          {
            setting: 'release',
            console: { worker: 'api', command: 'pnpm db:migrate --force' },
            file: { worker: 'api', command: 'pnpm db:migrate' },
          },
        ],
      },
    },
  },
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
  compose_files: [
    { path: 'api/worker-compose-prod.yaml', choosable: true, suggested_environment: null, issue: null, workers: 11 },
    { path: 'api/worker-compose.staging.yaml', choosable: true, suggested_environment: null, issue: null, workers: 11 },
    {
      path: 'api/workers-compose.yaml',
      choosable: false,
      suggested_environment: null,
      issue: 'NOT_WORKER_COMPOSE',
      workers: null,
    },
  ],
}

/** `GET /teams/{t}/environments/{dev}`: the release command was changed in the console. */
export const devEnvironment = {
  id: DEV.id,
  project_id: 'p1',
  name: 'dev',
  production: false,
  protected: false,
  branch: 'dev',
  manifest: 'api/worker-compose-prod.yaml',
  status: 'active',
  url: 'https://registry-2-dev.iii.cloud',
  settings: {
    build_command: null,
    runtimes: {},
    release: { worker: 'api', command: 'pnpm db:migrate --force' },
    healthcheck: { path: '/health', timeout: '10s' },
    web: { serve: 'container', container: 'http', port: 8080 },
    root_directory: null,
  },
  pending: [],
  from_file: ['web', 'healthcheck'],
  created_at: T(60 * 24 * 14),
  delete_after: null,
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

export const devVariables = {
  variables: [
    variable('STRIPE_SECRET_KEY', 'secret', 'set', 'after_restart:app,jobs', 2, { required_by: ['billing'] }),
    variable('GITHUB_CLIENT_SECRET', 'secret', 'set', 'after_restart:app', 60 * 13),
    variable('GITHUB_CLIENT_ID', 'variable', 'set', 'live', 60 * 13, { value: 'Iv1.8a61f9b3a7aba766' }),
    variable('SEARCH_WEIGHT_STARS', 'variable', 'set', null, 28, { value: '0.4' }),
    variable('ASSET_BUCKET_KEY', 'secret', 'empty_default', null, null),
  ],
}

export const DB_ID = '4b3a2f1e-0d9c-4b8a-a7f6-e5d4c3b2a1f0'
export const devResources = {
  resources: [
    {
      id: DB_ID,
      kind: 'database',
      alias: 'primary',
      mode: 'managed',
      status: 'ready',
      used_by: ['api'],
      size: 'small',
      tier: 'shared',
      backup_retention_days: 7,
      cors_origins: null,
      connection: [{ name: 'DATABASE_PRIMARY_URL', status: 'set', source: 'manifest' }],
      credentials: { rotated_at: null, rotating: false },
      snapshot: null,
      delete_after: null,
      created_at: T(60 * 14),
    },
  ],
}

const manageSettings: Board = {
  status: settingsStatus,
  context: { at: 'secrets', env: DEV.id },
  cloud: {
    [apiPath('', DEV.id)]: devEnvironment,
    [apiPath('/variables', DEV.id)]: devVariables,
    [apiPath('/resources', DEV.id)]: devResources,
  },
}

/** `GET …/config`: what iii cloud keeps per worker; `ui` fields were copied (or set) in iii cloud. */
const field = (key: string, value: unknown, source = 'ui') => ({
  key,
  value,
  draft: value,
  source,
  editable: source === 'ui',
  applies: 'restart:app',
})
export const devConfigView = {
  version: 4,
  draft_changed: false,
  workers: [
    {
      name: 'http',
      fields: [field('port', 3111), field('host', '0.0.0.0'), field('default_timeout', 30000, 'default')],
    },
    { name: 'queue', fields: [field('adapter.name', 'builtin')] },
    { name: 'state', fields: [field('adapter.name', 'kv', 'default')] },
  ],
}
const manageSettingsConfig: Board = {
  ...manageSettings,
  status: {
    ...settingsStatus,
    check: settingsStatus.check ? { ...settingsStatus.check, problems: [devConfig] } : null,
  },
  cloud: { ...manageSettings.cloud, [apiPath('/config', DEV.id)]: devConfigView },
  configImport: { ...configPreview, environment_id: DEV.id },
}

// Literal keys, like the other area modules: `board('manage-settings')` type-checks.
export const boards = { 'manage-settings': manageSettings, 'manage-settings-config': manageSettingsConfig }
