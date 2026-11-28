// Secrets and variables (Env4): every name the compose file reads, once; a row's one action; what is saved where;
// the restart notice only when a value reached iii cloud after the serving deployment started.
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { apiPath, request } from '../fixtures/base'
import { DEV, devSecretValues, devVariables, settingsStatus } from '../fixtures/settings'
import { valueItem } from '../fixtures/values'
import { when } from '../model'
import type { Status, Values } from '../types'
import type { Call } from './host'
import { board, btn, byText, click, cloudAnswers, ctx, el, flush, mount, text, type, ui, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

const settle = async () => {
  for (let i = 0; i < 5; i++) await flush()
}
const writes = (calls: Call[]) => calls.filter((c) => c.id === 'iii-cloud::api::write').map((c) => c.payload)
const of = (h: { calls: Call[] }, id: string) => h.calls.filter((c) => c.id === id).map((c) => c.payload)
const ok = (body: unknown = {}) => ({ status: 200, body })
const rows = () => [...el.querySelectorAll('[aria-label="Secrets and variables"] tbody tr')]
const rowOf = (name: string) => rows().find((r) => r.querySelector('td')?.textContent === name)
const cells = (name: string) => {
  const tds = [...(rowOf(name)?.querySelectorAll('td') ?? [])]
  // The kind cell holds its menu too: the word is its first node.
  return [
    tds[0]?.textContent,
    tds[1]?.querySelector('.iii-cloud-kind')?.firstChild?.textContent,
    tds[2]?.textContent,
    tds[3]?.textContent,
  ]
}
const action = (name: string) => rowOf(name)?.querySelector('td:last-child button')
const saveInAdd = () => [...el.querySelectorAll('.iii-cloud-set-add button')].find((b) => b.textContent === 'Save')
const path = () => el.querySelector('[role=toolbar] nav')?.textContent
const area = () => el.querySelector('[role=dialog] textarea') as HTMLTextAreaElement
async function typeArea(value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set?.call(area(), value)
    area().dispatchEvent(new Event('input', { bubbles: true }))
  })
}
const open = (
  over: Partial<Status> = {},
  values: Values = devSecretValues,
  extra: Record<string, unknown> = {},
  answers = {},
) =>
  mount({ ...settingsStatus, ...over }, ctx({ at: 'secrets', env: DEV.id }), {
    ...cloudAnswers({ [apiPath('/variables', DEV.id)]: devVariables, ...extra }),
    'iii-cloud::values': values,
    ...answers,
  })

describe('the list', () => {
  it('every name the compose file reads, once, and the other secrets and variables of the environment after them', async () => {
    const h = await board('manage-settings')
    await settle()
    expect(rows().map((r) => r.querySelector('td')?.textContent)).toEqual([
      'STRIPE_SECRET_KEY',
      'GITHUB_CLIENT_SECRET',
      'GITHUB_CLIENT_ID',
      'SEARCH_WEIGHT_STARS',
      'DATABASE_PRIMARY_URL',
      'ASSET_BUCKET_KEY',
      'LEGACY_FLAG',
    ])
    expect(path()).toBe('Projects / registry · dev / Secrets and variables')
    expect(el.querySelector('aside')).toBeNull()
    // No override banners, next-deploy table, workers config, machines or compose-file dialog, and none is read.
    for (const gone of [
      'Use my app file',
      'What the next deploy runs',
      "Workers' config",
      'Rotate credentials',
      'Change…',
      'Machine',
    ]) {
      expect(text()).not.toContain(gone)
    }
    const reads = h.calls.filter((c) => c.id === 'iii-cloud::api').map((c) => String(c.payload?.path))
    expect(reads).toContain(apiPath('/variables', DEV.id))
    expect(reads.filter((p) => /resources|config|machines|domains/.test(p) || p.endsWith(DEV.id))).toEqual([])
  })

  it('name, kind, value or where it came from, when it takes effect, and one action', async () => {
    await board('manage-settings')
    await settle()
    expect(cells('STRIPE_SECRET_KEY')).toEqual([
      'STRIPE_SECRET_KEY',
      'secret',
      expect.stringContaining('Set · '),
      'after app and jobs restart',
    ])
    expect(cells('GITHUB_CLIENT_SECRET')[3]).toBe('live')
    expect(cells('GITHUB_CLIENT_ID')).toEqual(['GITHUB_CLIENT_ID', 'variable', 'Iv1.8a61f9b3a7aba766', 'live'])
    expect(cells('SEARCH_WEIGHT_STARS')).toEqual(['SEARCH_WEIGHT_STARS', 'variable', '0.35', 'next deploy'])
    expect(cells('DATABASE_PRIMARY_URL')).toEqual([
      'DATABASE_PRIMARY_URL',
      'set by iii cloud',
      'iii cloud makes the database and fills this in. Nothing to do.',
      '',
    ])
    expect(cells('ASSET_BUCKET_KEY')).toEqual([
      'ASSET_BUCKET_KEY',
      'optional',
      'Not set. Your compose file gives it an empty default, so dev deploys without it.',
      '',
    ])
    expect(action('STRIPE_SECRET_KEY')?.textContent).toBe('Replace')
    expect(action('GITHUB_CLIENT_SECRET')?.textContent).toBe('Replace')
    expect(action('GITHUB_CLIENT_ID')?.textContent).toBe('Edit')
    expect(action('SEARCH_WEIGHT_STARS')?.textContent).toBe('Edit')
    expect(action('ASSET_BUCKET_KEY')?.textContent).toBe('Set')
    expect(action('DATABASE_PRIMARY_URL')).toBeNull()
    // Set on a name with no value: a typed value starts as a secret.
    await click(action('ASSET_BUCKET_KEY'))
    expect(el.querySelector('tbody input[type="password"]')).not.toBeNull()
  })

  it('what the file reads is said under the title, with what is not set', async () => {
    await board('manage-settings')
    await settle()
    expect(el.querySelector('.iii-cloud-set-head')?.textContent).toContain(
      "api/worker-compose-prod.yaml reads 6 names in dev · 1 optional one isn't set",
    )
    expect(text()).toContain('This is the only place that lists every name.')
    expect(text()).toContain("A name a worker needs can't be deleted: billing reads STRIPE_SECRET_KEY.")
  })

  it('a name the agent asked for that no file or variable lists is a row too', async () => {
    const ask = { ...request('NEW_TOKEN', 'ask', 'waiting', 'Used by the new worker.'), environment_id: DEV.id }
    await open({ secrets: [ask] })
    await settle()
    expect(cells('NEW_TOKEN')[2]).toBe('Asked by your agent: type it here.')
    expect(action('NEW_TOKEN')?.textContent).toBe('Set')
  })

  it('a value iii cloud could not give (its list failed): the names still list, and Retry is offered', async () => {
    await open(
      {},
      devSecretValues,
      {},
      { 'iii-cloud::api': () => ({ status: 500, body: { code: 'CLOUD_ERROR', message: '' } }) },
    )
    await settle()
    expect(text()).toContain("Couldn't read the secrets and variables from iii cloud")
    expect(rowOf('STRIPE_SECRET_KEY')).not.toBeUndefined()
  })

  it('nothing at all: an empty state that says what to do', async () => {
    await open({}, { ...devSecretValues, items: [] }, { [apiPath('/variables', DEV.id)]: { variables: [] } })
    await settle()
    expect(text()).toContain('No secrets or variables yet')
  })
})

describe('names iii cloud sets', () => {
  it('a database binding says the database is made; another built-in says it is filled at each deploy', async () => {
    const builtin = {
      name: 'IIICLOUD_GIT_BRANCH',
      type: 'builtin',
      value: null,
      status: 'set',
      required_by: [],
      applies: null,
      updated_at: null,
    }
    await open({}, devSecretValues, {
      [apiPath('/variables', DEV.id)]: { variables: [...devVariables.variables, builtin] },
    })
    await settle()
    expect(cells('IIICLOUD_GIT_BRANCH')[2]).toBe('iii cloud fills this in at each deploy. Nothing to do.')
    expect(cells('IIICLOUD_GIT_BRANCH')[3]).toBe('')
    expect(cells('DATABASE_PRIMARY_URL')[2]).toBe('iii cloud makes the database and fills this in. Nothing to do.')
  })
})

describe('the restart notice', () => {
  it('shows when a value reached iii cloud after the serving deployment started, with its two actions', async () => {
    const h = await board('manage-settings')
    await settle()
    const note = byText(
      '[role=status] b',
      /* the headline */ `STRIPE_SECRET_KEY changed at ${when(devVariables.variables[0].updated_at as string)}`,
    )
    expect(note).not.toBeUndefined()
    expect(note?.parentElement?.textContent).toContain(
      'The app and jobs groups keep the old value until they restart. The next deploy restarts them too.',
    )
    expect(btn('Restart app and jobs now')).not.toBeNull()
    expect(btn('Wait for the next deploy')).not.toBeNull()
    // Wait for the next deploy hides it without a call.
    await click(btn('Wait for the next deploy'))
    expect(text()).not.toContain('keep the old value')
    expect(writes(h.calls)).toEqual([])
  })

  it('Restart app and jobs now asks, then restarts each group as the person', async () => {
    const h = await board('manage-settings', {}, { 'iii-cloud::api::write': () => ok() })
    await settle()
    await click(btn('Restart app and jobs now'))
    expect(writes(h.calls)).toEqual([])
    await click(el.querySelector('[role=alertdialog] button'))
    await settle()
    expect(writes(h.calls).map((w) => [w?.method, w?.path])).toEqual([
      ['POST', apiPath('/machines/groups/app/restart', DEV.id)],
      ['POST', apiPath('/machines/groups/jobs/restart', DEV.id)],
    ])
    expect(text()).not.toContain('keep the old value')
  })

  it('not at all when nothing changed since the deployment started, or the value is live', async () => {
    const old = {
      variables: devVariables.variables.map((v) => ({ ...v, updated_at: v.updated_at && '2020-01-01T00:00:00Z' })),
    }
    await open({}, devSecretValues, { [apiPath('/variables', DEV.id)]: old })
    await settle()
    expect(text()).not.toContain('changed at')
    const live = { variables: devVariables.variables.map((v) => ({ ...v, applies: 'live' })) }
    await mount({ ...settingsStatus }, ctx({ at: 'secrets', env: DEV.id }, 2), {
      ...cloudAnswers({ [apiPath('/variables', DEV.id)]: live }),
      'iii-cloud::values': devSecretValues,
    })
    await settle()
    expect(text()).not.toContain('changed at')
  })
})

describe('saving from a row', () => {
  it('Replace on a secret: the value goes only to secret::provide, never another call', async () => {
    const h = await board('manage-settings', {}, { 'iii-cloud::secret': { id: 'sr-1' } })
    await settle()
    await click(action('STRIPE_SECRET_KEY'))
    const input = el.querySelector('input[type="password"]')
    expect(input).not.toBeNull()
    const value = 'sk_live_9f8e7d6c5b4a'
    await type(input, value)
    await click(btn('Save'))
    await settle()
    expect(of(h, 'iii-cloud::secret')).toEqual([{ name: 'STRIPE_SECRET_KEY', description: 'Read by billing.' }])
    expect(of(h, 'iii-cloud::secret::provide')).toEqual([{ request_id: 'sr-1', value }])
    expect(JSON.stringify(h.calls.filter((c) => !c.id.startsWith('iii-cloud::secret')))).not.toContain(value)
  })

  it('Edit on a variable: PUT with the new value; Save waits for a change', async () => {
    const h = await board('manage-settings', {}, { 'iii-cloud::api::write': () => ok() })
    await settle()
    await click(action('SEARCH_WEIGHT_STARS'))
    expect((btn('Save') as HTMLButtonElement).disabled).toBe(true)
    await type(el.querySelector('tbody input'), '0.5')
    await click(btn('Save'))
    await settle()
    expect(writes(h.calls)).toEqual([
      {
        method: 'PUT',
        path: apiPath('/variables/SEARCH_WEIGHT_STARS', DEV.id),
        body: { type: 'variable', value: '0.5' },
        idempotency_key: expect.any(String),
      },
    ])
  })

  it("the row's menu turns the kind around: a secret becomes a variable (and back), no two buttons per row", async () => {
    const h = await board('manage-settings', {}, { 'iii-cloud::api::write': () => ok() })
    await settle()
    expect(rowOf('STRIPE_SECRET_KEY')?.querySelectorAll('button')).toHaveLength(3)
    expect(btn('Secret')).toBeNull()
    await click(rowOf('STRIPE_SECRET_KEY')?.querySelector('[role=menu] button'))
    expect(text()).toContain('Not secret: everyone on the team can read a variable')
    await type(el.querySelector('tbody input[type="text"], tbody input:not([type])'), 'plain')
    await click(btn('Save'))
    await settle()
    expect(writes(h.calls)[0]).toMatchObject({ body: { type: 'variable', value: 'plain' } })
    // A variable's menu offers the other way.
    await click(rowOf('SEARCH_WEIGHT_STARS')?.querySelector('[role=menu] button'))
    expect(el.querySelector('tbody input[type="password"]')).not.toBeNull()
  })

  it("an agent's ask for the name is the one the value answers", async () => {
    const ask = { ...request('ASSET_BUCKET_KEY', 'ask', 'waiting', 'Where uploads go.'), environment_id: DEV.id }
    const h = await open({ secrets: [ask] }, devSecretValues, {}, { 'iii-cloud::secret::provide': {} })
    await settle()
    await click(action('ASSET_BUCKET_KEY'))
    await type(el.querySelector('input[type="password"]'), 'bucket-1')
    await click(btn('Save'))
    await settle()
    expect(of(h, 'iii-cloud::secret')).toEqual([])
    expect(of(h, 'iii-cloud::secret::provide')).toEqual([{ request_id: ask.id, value: 'bucket-1' }])
  })

  it('Delete asks first, and a name a worker needs cannot be deleted', async () => {
    const h = await board('manage-settings', {}, { 'iii-cloud::api::write': () => ok() })
    await settle()
    await click(action('GITHUB_CLIENT_ID'))
    await click(btn('Delete'))
    expect(writes(h.calls)).toEqual([])
    await click(el.querySelector('[role=alertdialog] button'))
    await settle()
    expect(writes(h.calls)[0]).toMatchObject({
      method: 'DELETE',
      path: apiPath('/variables/GITHUB_CLIENT_ID', DEV.id),
      query: { confirm: 'GITHUB_CLIENT_ID' },
    })
    await click(action('STRIPE_SECRET_KEY'))
    expect((btn('Delete') as HTMLButtonElement).disabled).toBe(true)
  })

  it("the cloud's refusal is shown as it says it", async () => {
    const forbidden = {
      status: 403,
      body: { code: 'FORBIDDEN', message: 'Only an Owner or Admin can change variables.' },
    }
    await board('manage-settings', {}, { 'iii-cloud::api::write': () => forbidden })
    await settle()
    await click(action('SEARCH_WEIGHT_STARS'))
    await type(el.querySelector('tbody input'), '1')
    await click(btn('Save'))
    await settle()
    expect(text()).toContain('Only an Owner or Admin can change variables.')
  })
})

describe('Add', () => {
  it('a secret first: the name is constrained, the value goes only to secret::provide', async () => {
    const h = await board('manage-settings', {}, { 'iii-cloud::secret': { id: 'sr-add' } })
    await settle()
    await click(btn('Add'))
    await type(el.querySelector('.iii-cloud-set-add input'), 'new token-1')
    expect(text()).toContain('NEWTOKEN1')
    const value = 'tok_9f8e7d6c5b4a'
    await type(el.querySelector('.iii-cloud-set-add input[type="password"]'), value)
    await click(saveInAdd())
    await settle()
    expect(of(h, 'iii-cloud::secret')).toEqual([
      { name: 'NEWTOKEN1', description: 'Added on the Secrets and variables page.' },
    ])
    expect(of(h, 'iii-cloud::secret::provide')).toEqual([{ request_id: 'sr-add', value }])
  })

  it('a variable is written with PUT', async () => {
    const h = await board('manage-settings', {}, { 'iii-cloud::api::write': () => ok() })
    await settle()
    await click(btn('Add'))
    const inputs = () => el.querySelectorAll('.iii-cloud-set-add input')
    await type(inputs()[0], 'LOG_LEVEL')
    await click(btn('Variable'))
    await type(inputs()[1], 'debug')
    await click(saveInAdd())
    await settle()
    expect(writes(h.calls)).toEqual([
      {
        method: 'PUT',
        path: apiPath('/variables/LOG_LEVEL', DEV.id),
        body: { type: 'variable', value: 'debug' },
        idempotency_key: expect.any(String),
      },
    ])
  })
})

describe('Paste a .env', () => {
  const pasted = [
    '# a comment',
    'STRIPE_SECRET_KEY=sk_live_new',
    'ASSET_BUCKET_KEY="b-1"',
    'DATABASE_PRIMARY_URL=postgres://x',
    'OTHER_NAME=1',
  ].join('\n')

  it('saves the names the file reads as secrets through values::save, and says what it left out', async () => {
    const h = await board(
      'manage-settings',
      {},
      {
        'iii-cloud::values::save': {
          environment_id: DEV.id,
          results: [
            { name: 'STRIPE_SECRET_KEY', ok: true },
            { name: 'ASSET_BUCKET_KEY', ok: true },
          ],
        },
      },
    )
    await settle()
    await click(btn('Paste a .env'))
    await typeArea(pasted)
    expect(el.querySelector('[role=dialog]')?.textContent).toContain(
      '2 names will be saved: STRIPE_SECRET_KEY, ASSET_BUCKET_KEY.',
    )
    expect(el.querySelector('[role=dialog]')?.textContent).toContain('skipped: OTHER_NAME')
    // STRIPE_SECRET_KEY is already set in dev: the person confirms the replacement before Save works.
    expect(el.querySelector('[role=dialog]')?.textContent).toContain(
      '1 name already set in dev will be replaced: STRIPE_SECRET_KEY.',
    )
    expect((btn('Save 2 secrets') as HTMLButtonElement).disabled).toBe(true)
    await click(el.querySelector('[role=dialog] input[type=checkbox]'))
    expect((btn('Save 2 secrets') as HTMLButtonElement).disabled).toBe(false)
    await click(btn('Save 2 secrets'))
    await settle()
    expect(of(h, 'iii-cloud::values::save')).toEqual([
      {
        environment_id: DEV.id,
        manifest: 'api/worker-compose-prod.yaml',
        items: [
          { name: 'STRIPE_SECRET_KEY', from: 'typed', type: 'secret', value: 'sk_live_new' },
          { name: 'ASSET_BUCKET_KEY', from: 'typed', type: 'secret', value: 'b-1' },
        ],
      },
    ])
    // All saved: the dialog closes, and the pasted text is gone with it.
    expect(el.querySelector('[role=dialog]')).toBeNull()
    expect(text()).not.toContain('sk_live_new')
  })

  it('a name that did not save stays in the box with why; what saved is done', async () => {
    await board(
      'manage-settings',
      {},
      {
        'iii-cloud::values::save': {
          environment_id: DEV.id,
          results: [
            { name: 'STRIPE_SECRET_KEY', ok: true },
            { name: 'ASSET_BUCKET_KEY', ok: false, error: { code: 'NOT_SAVED', message: "iii cloud didn't take it." } },
          ],
        },
      },
    )
    await settle()
    await click(btn('Paste a .env'))
    await typeArea(pasted)
    await click(el.querySelector('[role=dialog] input[type=checkbox]'))
    await click(btn('Save 2 secrets'))
    await settle()
    expect(el.querySelector('[role=dialog]')?.textContent).toContain(
      "ASSET_BUCKET_KEY did not saveiii cloud didn't take it.",
    )
    expect(area().value).toBe('ASSET_BUCKET_KEY=b-1')
  })

  it('is off while there is no list of names to read', async () => {
    await open(
      {},
      devSecretValues,
      {},
      {
        'iii-cloud::values': () => {
          throw new Error('down')
        },
      },
    )
    await settle()
    expect((btn('Paste a .env') as HTMLButtonElement).disabled).toBe(true)
  })
})

describe('a key that cannot write, or a role that cannot', () => {
  it('an API key: the list is read-only and points at iii cloud', async () => {
    await open({}, { ...devSecretValues, save: 'api_key' })
    await settle()
    expect(text()).toContain("An API key can't write secrets and variables.")
    expect((action('STRIPE_SECRET_KEY') as HTMLButtonElement).disabled).toBe(true)
    expect((btn('Add') as HTMLButtonElement).disabled).toBe(true)
    expect((btn('Paste a .env') as HTMLButtonElement).disabled).toBe(true)
  })

  it('production without Owner or Admin says so', async () => {
    await open({}, { ...devSecretValues, save: 'role' })
    await settle()
    expect(text()).toContain("Changing production's secrets and variables takes Owner or Admin.")
  })
})

describe('values that can come from env files', () => {
  const file = (f: string, preselect: boolean) => ({ source: 'env_file' as const, file: f, preselect, local: false })
  const offered: Values = {
    ...devSecretValues,
    items: [
      ...devSecretValues.items.filter((i) => i.name !== 'ASSET_BUCKET_KEY'),
      valueItem('ASSET_BUCKET_KEY', {
        containers: ['search'],
        suggestions: [file('api/.env.test', true), file('api/.env', false)],
      }),
      valueItem('PUBLIC_URL', {
        containers: ['search'],
        type: 'variable',
        suggestions: [{ source: 'env_url', url: 'https://dev.example.test', preselect: true }],
      }),
      valueItem('NOTHING', { containers: ['search'] }),
    ],
  }
  const saved = (names: string[]) => ({
    'iii-cloud::values::save': { environment_id: DEV.id, results: names.map((name) => ({ name, ok: true })) },
  })

  it('one row above the table counts the names with a suggestion; absent without any', async () => {
    await open({}, offered)
    await settle()
    expect(text()).toContain('2 values can come from your env files')
    expect(btn('Review and save')).toBeTruthy()
    await board('manage-settings')
    await settle()
    expect(text()).not.toContain('can come from your env files')
    expect(btn('Review and save')).toBeNull()
  })

  it('Review and save opens the review: it sends env_file and env_url items and never a value', async () => {
    const h = await open({}, offered, {}, saved(['ASSET_BUCKET_KEY', 'PUBLIC_URL']))
    await settle()
    await click(btn('Review and save'))
    expect(of(h, 'iii-cloud::values::save')).toEqual([])
    expect(el.querySelector('[role=dialog]')?.textContent).toContain('From api/.env.test')
    await click(btn('Save 2 values'))
    await settle()
    expect(of(h, 'iii-cloud::values::save')).toEqual([
      {
        environment_id: DEV.id,
        manifest: 'api/worker-compose-prod.yaml',
        items: [
          { name: 'ASSET_BUCKET_KEY', from: 'env_file', type: 'secret', file: 'api/.env.test' },
          { name: 'PUBLIC_URL', from: 'env_url' },
        ],
      },
    ])
    expect(el.querySelector('[role=dialog]')).toBeNull()
  })

  it('an item that failed stays in the review with its reason; the others are saved', async () => {
    const h = await open(
      {},
      offered,
      {},
      {
        'iii-cloud::values::save': {
          environment_id: DEV.id,
          results: [
            { name: 'ASSET_BUCKET_KEY', ok: false, error: { code: 'FILE_CHANGED', message: 'gone' } },
            { name: 'PUBLIC_URL', ok: true },
          ],
        },
      },
    )
    await settle()
    await click(btn('Review and save'))
    await click(btn('Save 2 values'))
    await settle()
    expect(of(h, 'iii-cloud::values::save')).toHaveLength(1)
    const dialog = el.querySelector('[role=dialog]')
    expect(dialog?.textContent).toContain('api/.env.test no longer has ASSET_BUCKET_KEY')
  })

  it("a row's editor offers only the preselected sources; a click selects, Save writes once", async () => {
    const h = await open({}, offered, {}, saved(['ASSET_BUCKET_KEY']))
    await settle()
    await click(btn('Set ASSET_BUCKET_KEY'))
    // api/.env is not preselected by the worker: only the review dialog offers it.
    expect(btn('From api/.env')).toBeNull()
    await click(btn('From api/.env.test'))
    expect(of(h, 'iii-cloud::values::save')).toEqual([])
    const input = el.querySelector('input') as HTMLInputElement
    expect(input.value).toBe('From api/.env.test')
    // Cancel writes nothing and brings the choices back.
    await click(btn('Cancel'))
    expect(of(h, 'iii-cloud::values::save')).toEqual([])
    await click(btn('From api/.env.test'))
    await click(btn('Save'))
    await settle()
    expect(of(h, 'iii-cloud::values::save')).toEqual([
      {
        environment_id: DEV.id,
        manifest: 'api/worker-compose-prod.yaml',
        items: [{ name: 'ASSET_BUCKET_KEY', from: 'env_file', type: 'secret', file: 'api/.env.test' }],
      },
    ])
    await click(btn('Set PUBLIC_URL'))
    await click(btn("This environment's address"))
    expect(of(h, 'iii-cloud::values::save')).toHaveLength(1)
    await click(btn('Save'))
    await settle()
    expect(of(h, 'iii-cloud::values::save')[1]).toMatchObject({ items: [{ name: 'PUBLIC_URL', from: 'env_url' }] })
  })

  it('no choices on a row that is set, or whose only source is not preselected', async () => {
    const values: Values = {
      ...offered,
      items: [
        ...offered.items.filter((i) => i.name !== 'STRIPE_SECRET_KEY' && i.name !== 'ASSET_BUCKET_KEY'),
        valueItem('STRIPE_SECRET_KEY', {
          containers: ['search'],
          in_cloud: 'set',
          suggestions: [file('api/.env', false)],
        }),
        valueItem('ASSET_BUCKET_KEY', { containers: ['search'], suggestions: [file('api/.env', false)] }),
      ],
    }
    const h = await open({}, values)
    await settle()
    await click(btn('Replace STRIPE_SECRET_KEY'))
    expect(btn('From api/.env')).toBeNull()
    await click(btn('Set ASSET_BUCKET_KEY'))
    expect(btn('From api/.env')).toBeNull()
    expect(of(h, 'iii-cloud::values::save')).toEqual([])
  })

  it('the count is what the dialog offers: names with a default and a localhost-only value are not counted', async () => {
    const values: Values = {
      ...devSecretValues,
      items: [
        ...devSecretValues.items.filter((i) => i.name !== 'ASSET_BUCKET_KEY'),
        valueItem('LOG_LEVEL', {
          containers: ['search'],
          need: 'has_default',
          default: 'info',
          suggestions: [file('api/.env', true)],
        }),
        valueItem('LOCAL_ONLY', { containers: ['search'], suggestions: [file('api/.env', false)] }),
        valueItem('NORMAL', { containers: ['search'], suggestions: [file('api/.env.test', true)] }),
      ],
    }
    const h = await open({}, values, {}, saved(['NORMAL']))
    await settle()
    expect(text()).toContain('1 value can come from your env files')
    await click(btn('Review and save'))
    // The description is this screen's, not the check's "local run" sentence.
    const dialog = el.querySelector('[role=dialog]')?.textContent ?? ''
    expect(dialog).toContain('saved to dev. Nothing is written until you save.')
    expect(dialog).not.toContain('The local run needs')
    expect(btn('Save 1 value')).toBeTruthy()
    expect(of(h, 'iii-cloud::values::save')).toEqual([])
  })

  it('a connection that cannot write sees the row disabled', async () => {
    const h = await open({}, { ...offered, save: 'api_key' })
    await settle()
    expect(text()).toContain('2 values can come from your env files')
    expect((btn('Review and save') as HTMLButtonElement).disabled).toBe(true)
    expect(of(h, 'iii-cloud::values::save')).toEqual([])
  })
})

describe('on a narrow pane', () => {
  it('rows are a list with the kind, the value and the action', async () => {
    ui.narrow = true
    await board('manage-settings')
    await settle()
    const row = [...el.querySelectorAll('[role=row]')].find((r) => r.textContent?.startsWith('SEARCH_WEIGHT_STARS'))
    expect(row?.textContent).toContain('variable · 0.35 · next deploy')
    expect(row?.querySelector('button')?.textContent).toBe('Edit')
  })
})
