// What `iii-cloud::values` answers (VALUES-PLAN §2): release-control · test reads 22 names, 21 of them not set
// (the live check's 21 empty defaults, lines 81–101, plus RELEASE_SHA with a default), and registry · dev, where
// three names of the folder's file aren't set. Never a secret's value: file names and flags only.

import type { Status, Suggestion, ValueItem, Values } from '../types'
import type { Board } from './base'
import { boards as checkBoards, RC_HERE_UNSET, RC_SKIPPED, RC_VARIABLES, rcFixing, TEST_ENV } from './check'
import { DEV, boards as deployBoards } from './deploy'

export const RC_PROD = { id: '8d1f4b6a-2c3e-4f5a-9b8c-7d6e5f4a3b2c', name: 'production' }
export const RC_URL = 'https://test.release-control.iii.app'
const CONTAINER = 'worker-release-control'

const envFile = (file: string, preselect: boolean, local = false): Suggestion => ({
  source: 'env_file',
  file,
  preselect,
  local,
})
const copy: Suggestion = { source: 'copy', environment_id: RC_PROD.id, environment: RC_PROD.name, preselect: false }

/** One name of the live file: `${NAME:-}` on its line, read by the one container, a secret until said otherwise. */
export const valueItem = (name: string, over: Partial<ValueItem> = {}): ValueItem => ({
  name,
  need: 'optional',
  default: '',
  type: 'secret',
  database: false,
  containers: [CONTAINER],
  line: 81 + RC_VARIABLES.indexOf(name),
  in_cloud: 'unset',
  value: null,
  updated_at: null,
  asked: null,
  suggestions: [],
  ...over,
})

const fromTest = [envFile('.env.test', true), envFile('.env.production', false)]
/** The run here already has it: from the folder's `.env.test`. */
const hereFromTest: Partial<ValueItem> = { here: 'from_file', here_file: 'api/.env.test' }
const SOURCES: Record<string, Partial<ValueItem>> = {
  APP_BASE_URL: {
    type: 'variable',
    suggestions: [{ source: 'env_url', url: RC_URL, preselect: true }, envFile('.env.production', false)],
  },
  DATABASE_URL: { database: true },
  HARNESS_E2E_OIDC_AUDIENCE: { suggestions: fromTest, ...hereFromTest },
  HARNESS_E2E_WORKFLOW_FILE: { suggestions: fromTest, ...hereFromTest },
  HARNESS_E2E_WORKFLOW_REF: { suggestions: fromTest, ...hereFromTest },
  RELEASE_NOTES_URL: { suggestions: [envFile('.env.local', false, true)] },
  SENTRY_DSN: { suggestions: [copy] },
  SESSION_SECRET: {
    asked: { request_id: 'sr-rc-session', description: 'Signs the session cookies of the web app.' },
  },
  SLACK_BOT_TOKEN: { suggestions: [copy] },
  SLACK_CHANNEL: { suggestions: [copy] },
}

export const rcValues: Values = {
  environment_id: TEST_ENV.id,
  environment: TEST_ENV.name,
  manifest: 'api/worker-compose-prod.yaml',
  read_from: 'workspace',
  cloud: 'read',
  save: 'ok',
  items: [
    ...RC_VARIABLES.map((name) =>
      valueItem(name, SOURCES[name] ?? (name.startsWith('GH_') ? { suggestions: fromTest } : {})),
    ),
    valueItem('RELEASE_SHA', { need: 'has_default', default: 'development', type: 'variable', line: 102 }),
  ],
}

/** registry · dev, the folder's own file: three of its names have no value in dev. */
export const devValues: Values = {
  environment_id: DEV.id,
  environment: DEV.name,
  manifest: 'api/worker-compose-prod.yaml',
  read_from: 'checkout',
  cloud: 'read',
  save: 'ok',
  items: [
    valueItem('STRIPE_SECRET_KEY', { need: 'required', default: null, containers: ['billing'], line: 31 }),
    valueItem('SEARCH_WEIGHT_STARS', {
      type: 'variable',
      containers: ['search'],
      line: 44,
      suggestions: [envFile('.env', true)],
    }),
    valueItem('SENTRY_DSN', {
      containers: ['search', 'billing'],
      line: 45,
      suggestions: [envFile('.env.local', false, true)],
    }),
    valueItem('GITHUB_CLIENT_ID', {
      type: 'variable',
      containers: ['search'],
      line: 46,
      in_cloud: 'set',
      value: 'Iv1.8a61f9b3a8',
      updated_at: new Date(Date.now() - 13 * 3_600_000).toISOString(),
    }),
  ],
}

/** registry · dev live and healthy: what is not set is optional (`${X:-}`), so it defaults to empty. */
export const devOptional: Values = {
  ...devValues,
  items: devValues.items.map((i) => (i.need === 'required' ? { ...i, in_cloud: 'set' as const } : i)),
}

/** Setup step 4, Values: release-control · test selected, nothing checked yet. */
const rcValuesStep: Status = { ...rcFixing, check: null, checks: [], agent: null, handoff_brief: null }

const ago = (min: number) => new Date(Date.now() - min * 60_000).toISOString()
/** Every name set in test two minutes ago (the Onb3d "All set" state). */
const rcAllSet: Values = {
  ...rcValues,
  items: rcValues.items.map((i) => ({
    ...i,
    in_cloud: 'set',
    updated_at: ago(2),
    value: i.type === 'variable' ? (i.name === 'APP_BASE_URL' ? RC_URL : (i.default ?? '')) : null,
  })),
}
/** Production, read by a Member: two set, the rest not (the Onb3d "Production" state). */
const rcRole: Values = {
  ...rcValues,
  environment_id: RC_PROD.id,
  environment: RC_PROD.name,
  save: 'role',
  items: rcValues.items.map((i) =>
    i.name === 'APP_BASE_URL'
      ? { ...i, in_cloud: 'set', value: 'https://release-control.iii.cloud', updated_at: ago(3 * 1440) }
      : i.name === 'SESSION_SECRET'
        ? { ...i, in_cloud: 'set', asked: null, updated_at: ago(12 * 1440) }
        : i,
  ),
}
/** P3: what the run here lacks (required names, no default), as the check names them. */
export const rcHereValues: Values = {
  ...rcValues,
  items: rcValues.items.map((i) =>
    RC_HERE_UNSET.includes(i.name) ? { ...i, need: 'required', default: null, here: 'unset' } : i,
  ),
}
/** SKIP-VALUES: the two names skipped for now. */
export const rcSkippedValues: Values = {
  ...rcValues,
  items: rcValues.items.map((i) => (RC_SKIPPED.includes(i.name) ? { ...i, skipped: true } : i)),
}
const valuesAt: Board = { status: rcValuesStep, context: { at: 'setup', step: 'values' } }

/** The Values step, and what `values` answers on two boards of other areas (index.ts spreads these last). */
export const boards = {
  'setup-values': { ...valuesAt, values: rcValues },
  'setup-values-allset': { ...valuesAt, values: rcAllSet },
  'setup-values-apikey': { ...valuesAt, values: { ...rcValues, save: 'api_key' } },
  // Production selected: the page drops values read for another environment than the selected one.
  'setup-values-role': {
    ...valuesAt,
    status: {
      ...rcValuesStep,
      project: rcValuesStep.project && {
        ...rcValuesStep.project,
        environment: rcValuesStep.project.environment && {
          ...rcValuesStep.project.environment,
          id: RC_PROD.id,
          name: RC_PROD.name,
        },
      },
    },
    values: rcRole,
  },
  'setup-fixing-live': { ...checkBoards['setup-fixing-live'], values: rcValues },
  'setup-fixing-here': { ...checkBoards['setup-fixing-here'], values: rcHereValues },
  'setup-fixing-skipped': { ...checkBoards['setup-fixing-skipped'], values: rcSkippedValues },
  'setup-fixing-secret-skipped': { ...checkBoards['setup-fixing-secret-skipped'], values: rcSkippedValues },
  'setup-values-skipped': { ...valuesAt, values: rcSkippedValues },
  'deploy-problems': { ...checkBoards['deploy-problems'], values: devValues },
  'deploy-what': { ...deployBoards['deploy-what'], values: devValues },
  'deploy-what-optional': { ...deployBoards['deploy-what'], values: devOptional },
  // SKIP-VALUES: one of the two optional names skipped.
  'deploy-what-skipped': {
    ...deployBoards['deploy-what'],
    values: { ...devOptional, items: devOptional.items.map((i, n) => (n === 1 ? { ...i, skipped: true } : i)) },
  },
} satisfies Record<string, Board>
