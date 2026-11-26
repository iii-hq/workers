// The Settings tab (U8): what the agent asks for, console overrides, what the next deploy runs,
// secrets and variables (Mng3) and the change-file dialog (Mng11).
import { act } from 'react'
import { afterEach, beforeEach, describe, expect, it, type MockInstance, vi } from 'vitest'
import { boards } from '../fixtures'
import { apiPath, base, check, ENV, problems, secretRequests, variables } from '../fixtures/base'
import { DB_ID, DEV, devEnvironment } from '../fixtures/settings'
import { rcValues, valueItem } from '../fixtures/values'
import type { Status, Values } from '../types'
import type { Call } from './host'
import { board, btn, click, cloudAnswers, ctx, el, flush, mount, row, text, type, ui, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

const settings = ctx({ tab: 'secrets' })
/** production with one problem left: the agent asked for SESSION_SECRET and noted three more. */
const secretBoard: Status = {
  ...base,
  check: {
    ...check(false, false),
    problems: problems.slice(0, 1),
    here: { problems: 1, iii_version: '0.27.0', accepts_cloud_block: true },
    plan: check(true, false).plan,
  },
  secrets: secretRequests,
  variables,
}
const writes = (calls: Call[]) => calls.filter((c) => c.id === 'iii-cloud::api::write').map((c) => c.payload)
const variableRows = () =>
  [...el.querySelectorAll('[aria-label="Secrets and variables"] tbody tr')].map((r) =>
    [...r.querySelectorAll('td')].map((td) => td.textContent),
  )
const settingRows = () =>
  [...el.querySelectorAll('[aria-label="What the next deploy runs"] tbody tr')].map((r) =>
    [...r.querySelectorAll('td')].map((td) => td.textContent),
  )
const dialogConfirm = () => click(el.querySelector('[role=alertdialog] button'))
const ok = (body: unknown = {}) => ({ status: 200, body })
const of = (h: { calls: Call[] }, id: string) => h.calls.filter((c) => c.id === id).map((c) => c.payload)
/** Picks an option of a Select (the mock renders a <select>). */
async function pick(label: string, value: string) {
  const select = el.querySelector(`select[aria-label="${label}"]`) as HTMLSelectElement
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value')?.set?.call(select, value)
    select.dispatchEvent(new Event('change', { bubbles: true }))
  })
}

/** Secret values: only `secret::provide` carries one, never another call, a console method or the page. */
function secretWatch() {
  let spies: MockInstance[] = []
  beforeEach(() => {
    spies = (['log', 'info', 'warn', 'error', 'debug'] as const).map((m) =>
      vi.spyOn(console, m).mockImplementation(() => undefined),
    )
  })
  afterEach(() => {
    for (const s of spies) s.mockRestore()
  })
  return (calls: Call[], value: string) => {
    const carrying = calls.filter((c) => JSON.stringify(c).includes(value))
    expect(carrying.map((c) => c.id)).toEqual(['iii-cloud::secret::provide'])
    for (const s of spies) expect(JSON.stringify(s.mock.calls)).not.toContain(value)
    expect(text()).not.toContain(value)
  }
}

/** What `values` answers for production: the file reads SESSION_SECRET (the agent asked for it) and SUPABASE_URL. */
const fileValues: Values = {
  ...rcValues,
  environment_id: ENV.id,
  environment: ENV.name,
  items: [
    valueItem('SESSION_SECRET', {
      containers: ['registry'],
      line: 12,
      asked: { request_id: 'sr-SESSION_SECRET', description: secretRequests[0].description },
    }),
    valueItem('SUPABASE_URL', { type: 'variable', containers: ['registry'], line: 13 }),
  ],
}
const editor = () => el.querySelector('[aria-label="Values your app file reads"]')

describe('what the agent asks for: rows of the values editor', () => {
  it("the file's names are the editor's rows, the agent's ask among them; the other names stay below", async () => {
    const h = await mount(secretBoard, settings, { 'iii-cloud::values': fileValues })
    expect(of(h, 'iii-cloud::values')).toEqual([{}])
    const rows = [...(editor()?.querySelectorAll('.iii-cloud-val-row') ?? [])].map((r) => r.getAttribute('data-name'))
    expect(rows).toEqual(['SESSION_SECRET', 'SUPABASE_URL'])
    expect(editor()?.textContent).toContain('Asked by your agent')
    expect(editor()?.textContent).toContain(secretRequests[0].description)
    // Settings lists every name: no "Not set" filter.
    expect(editor()?.textContent).not.toContain('Not set (2)')
    // The Asked panel is gone: its asks are the editor's rows.
    expect(text()).not.toContain('Waiting for you')
    expect(variableRows().map((r) => [r[0], r[1]])).toEqual([
      ['DATABASE_URL', 'set by iii cloud'],
      ['SUPABASE_ANON_KEY', 'secret · your decision'],
      ['OPENAI_API_KEY', 'secret · not needed'],
    ])
    expect(text()).toContain("Couldn't read the variables from iii cloud")
  })

  it('without an answer from values, the panel is not there and every name stays below', async () => {
    await mount(secretBoard, settings)
    expect(editor()).toBeNull()
    expect(variableRows().map((r) => r[0])).toEqual([
      'DATABASE_URL',
      'SESSION_SECRET',
      'SUPABASE_URL',
      'SUPABASE_ANON_KEY',
      'OPENAI_API_KEY',
    ])
  })

  it('Leave empty on an optional name saves it with values::save, no check', async () => {
    const saved = { environment_id: ENV.id, results: [{ name: 'SUPABASE_URL', ok: true }] }
    const h = await mount(secretBoard, settings, { 'iii-cloud::values': fileValues, 'iii-cloud::values::save': saved })
    await pick('Where SUPABASE_URL comes from', 'leave_empty')
    await click(btn('Save 1 value'))
    await flush()
    expect(of(h, 'iii-cloud::values::save')).toEqual([
      { environment_id: ENV.id, manifest: fileValues.manifest, items: [{ name: 'SUPABASE_URL', from: 'leave_empty' }] },
    ])
    expect(of(h, 'iii-cloud::check')).toEqual([])
    expect(of(h, 'iii-cloud::secret::provide')).toEqual([])
  })

  it("an API key: the editor is read-only and points at iii cloud's console", async () => {
    const keyed = {
      ...secretBoard,
      connection: {
        ...secretBoard.connection,
        method: 'api_key' as const,
        api_key: { secret: 'K', team_id: 't', environment_id: 'e' },
      },
    }
    await mount(keyed, settings, { 'iii-cloud::values': { ...fileValues, save: 'api_key' } })
    expect(editor()?.textContent).toContain("An API key can't write values.")
    expect(editor()?.querySelector('a[href$="/iii/registry/production/variables"]')?.textContent).toBe(
      "Open production's values in iii cloud",
    )
    expect(editor()?.querySelector('input, select')).toBeNull()
    expect(btn('Sign in with your browser')).toBeTruthy()
  })

  it('says why a tracked .env was not written', async () => {
    const r = { ...secretBoard.secrets[0], state: 'set' as const, local: 'skipped' as const }
    await mount({ ...secretBoard, secrets: [r] }, settings)
    expect(text()).toContain('Not written to .env: the file is tracked by git')
    expect(text()).not.toContain('Asked by your agent')
  })

  it('on a phone, variables are stacked rows, not a table', async () => {
    ui.narrow = true
    await mount(secretBoard, settings)
    expect(el.querySelector('table')).toBeNull()
    expect(row('DATABASE_URL')?.textContent).toContain('set by iii cloud')
  })

  it('a variable that is empty by default says it is optional', async () => {
    const v = [
      { name: 'SENTRY_DSN', type: 'variable' as const, status: 'empty_default' as const, required_by: ['registry'] },
    ]
    await mount({ ...secretBoard, secrets: [], variables: v }, settings)
    expect(variableRows().find((r) => r[0] === 'SENTRY_DSN')?.[1]).toBe('optional · not set')
  })

  it("another environment's ask waits on that environment's page", async () => {
    const other = { ...secretBoard.secrets[0], environment_id: DEV.id }
    await mount({ ...secretBoard, secrets: [other] }, settings)
    expect(text()).not.toContain('Waiting for you')
  })

  describe('secret values', () => {
    secretWatch()
    it("the ask's value reaches only values::save: no other call and no console method carries it", async () => {
      const saved = { environment_id: ENV.id, results: [{ name: 'SESSION_SECRET', ok: true }] }
      const h = await mount(secretBoard, settings, {
        'iii-cloud::values': fileValues,
        'iii-cloud::values::save': saved,
      })
      const input = el.querySelector('input[aria-label="Value of SESSION_SECRET"]') as HTMLInputElement
      expect(input.type).toBe('password')
      const value = 'sup3r-secret-value-9f2'
      await type(input, value)
      await click(btn('Save 1 value'))
      await flush()
      expect(h.calls.filter((c) => JSON.stringify(c).includes(value)).map((c) => c.id)).toEqual([
        'iii-cloud::values::save',
      ])
      for (const m of ['log', 'info', 'warn', 'error', 'debug'] as const)
        expect(JSON.stringify((console[m] as unknown as MockInstance).mock.calls)).not.toContain(value)
      expect(el.innerHTML).not.toContain(value)
      expect(
        (el.querySelector('input[aria-label="Value of SESSION_SECRET"]') as HTMLInputElement | null)?.value ?? '',
      ).toBe('')
    })
  })
})

describe('manage-settings (Mng3)', () => {
  it('shows the console override, what the next deploy runs and every name', async () => {
    await board('manage-settings')
    expect(text()).toContain('The release command on dev was changed in the iii cloud console')
    expect(text()).toContain('iii cloud console: pnpm db:migrate --force · your app file: pnpm db:migrate.')
    expect(settingRows()).toEqual([
      ['App file', 'api/worker-compose-prod.yamlChange…', 'environment'],
      ['Release command', 'pnpm db:migrate --force', 'iii cloud console'],
      ['Web traffic', 'the http container, port 8080', 'app file'],
      ['Health check', '/health · timeout 10s', 'app file'],
      ['Database', 'Postgres primary, shared', 'iii cloud'],
    ])
    expect(variableRows()).toEqual([
      ['STRIPE_SECRET_KEY', 'secret', 'after app and jobs restart', '2m ago', 'Replace'],
      ['GITHUB_CLIENT_SECRET', 'secret', 'after app restarts', '13h ago', 'Replace'],
      ['GITHUB_CLIENT_ID', 'variable', 'live', '13h ago', 'Edit'],
      ['SEARCH_WEIGHT_STARS', 'variable', 'next deploy', '28m ago', 'Edit'],
      ['ASSET_BUCKET_KEY', 'optional · not set', '—', '—', 'Set'],
      ['DATABASE_PRIMARY_URL', 'managed by iii cloud', '—', '14h ago', 'Rotate credentials'],
    ])
    expect(text()).toContain("A name a worker needs can't be deleted: billing reads STRIPE_SECRET_KEY.")
  })

  it('"Use my app file" asks first, naming the setting and both values; PATCH "" only once confirmed', async () => {
    let env: unknown = devEnvironment
    const read = cloudAnswers(boards['manage-settings'].cloud ?? {})['iii-cloud::api']
    const h = await board(
      'manage-settings',
      {},
      {
        'iii-cloud::api': (p: Record<string, unknown>) => (p.path === apiPath('', DEV.id) ? ok(env) : read(p)),
        'iii-cloud::api::write': () => {
          env = { ...devEnvironment, settings: { ...devEnvironment.settings, release: null } }
          return ok(env)
        },
      },
    )
    await click(btn('Use my app file (recommended)'))
    await flush()
    expect(writes(h.calls)).toEqual([])
    const dialog = el.querySelector('[role=alertdialog]')?.textContent
    expect(dialog).toContain("Use your app file's release command on dev?")
    expect(dialog).toContain(
      "iii cloud removes the console's value, pnpm db:migrate --force. The next deploy of dev runs your app file's, pnpm db:migrate.",
    )
    await dialogConfirm()
    await flush()
    expect(el.querySelector('[role=alertdialog]')).toBeNull()
    expect(writes(h.calls)).toEqual([
      {
        method: 'PATCH',
        path: apiPath('', DEV.id),
        body: { settings: { release: null } },
        idempotency_key: expect.any(String),
      },
    ])
    expect(text()).not.toContain('was changed in the iii cloud console')
  })

  it('"Not now" dismisses it without a call', async () => {
    const h = await board('manage-settings')
    const before = h.calls.length
    await click(btn('Not now'))
    expect(h.calls.length).toBe(before)
    expect(text()).not.toContain('was changed in the iii cloud console')
  })

  it("shows the cloud's 403 message as it is", async () => {
    await board(
      'manage-settings',
      {},
      {
        'iii-cloud::api::write': () => ({
          status: 403,
          body: { code: 'FORBIDDEN', message: 'This needs the Admin role.' },
        }),
      },
    )
    await click(btn('Use my app file (recommended)'))
    await dialogConfirm()
    await flush()
    expect(text()).toContain('This needs the Admin role.')
  })

  it('Edit writes a variable with PUT /variables/{name} {type: variable}', async () => {
    const h = await board('manage-settings', {}, { 'iii-cloud::api::write': () => ok() })
    await click(el.querySelector('button[aria-label="Edit SEARCH_WEIGHT_STARS"]'))
    const input = el.querySelector('[aria-label="Secrets and variables"] input') as HTMLInputElement
    expect(input.value).toBe('0.4')
    await type(input, '0.6')
    await click(btn('Save'))
    await flush()
    expect(writes(h.calls)).toEqual([
      {
        method: 'PUT',
        path: apiPath('/variables/SEARCH_WEIGHT_STARS', DEV.id),
        body: { type: 'variable', value: '0.6' },
        idempotency_key: expect.any(String),
      },
    ])
  })

  it("the file's names are in the editor; the other names stay in the table below, with Delete", async () => {
    const read: Values = {
      ...rcValues,
      environment_id: DEV.id,
      environment: DEV.name,
      items: [
        valueItem('STRIPE_SECRET_KEY', {
          containers: ['billing'],
          in_cloud: 'set',
          updated_at: new Date().toISOString(),
        }),
        valueItem('GITHUB_CLIENT_ID', { type: 'variable', containers: ['search'], in_cloud: 'set', value: 'Iv1.8a61' }),
      ],
    }
    await board('manage-settings', {}, { 'iii-cloud::values': read })
    expect(
      [...(editor()?.querySelectorAll('.iii-cloud-val-row') ?? [])].map((r) => r.getAttribute('data-name')),
    ).toEqual(['STRIPE_SECRET_KEY', 'GITHUB_CLIENT_ID'])
    expect(editor()?.textContent).toContain('Iv1.8a61')
    expect(variableRows().map((r) => r[0])).toEqual([
      'GITHUB_CLIENT_SECRET',
      'SEARCH_WEIGHT_STARS',
      'ASSET_BUCKET_KEY',
      'DATABASE_PRIMARY_URL',
    ])
    await click(el.querySelector('button[aria-label="Edit SEARCH_WEIGHT_STARS"]'))
    expect(btn('Delete')).toBeTruthy()
    expect(btn('Rotate credentials')).toBeTruthy()
  })

  it('Delete is locked for a name a worker reads; otherwise it asks, then deletes with confirm', async () => {
    const h = await board('manage-settings', {}, { 'iii-cloud::api::write': () => ok() })
    await click(el.querySelector('button[aria-label="Replace STRIPE_SECRET_KEY"]'))
    expect(btn('Delete')?.hasAttribute('disabled')).toBe(true)
    await click(el.querySelector('button[aria-label="Edit GITHUB_CLIENT_ID"]'))
    expect(btn('Delete')?.hasAttribute('disabled')).toBe(false)
    await click(btn('Delete'))
    expect(writes(h.calls)).toEqual([])
    await dialogConfirm()
    await flush()
    expect(writes(h.calls)).toEqual([
      {
        method: 'DELETE',
        path: apiPath('/variables/GITHUB_CLIENT_ID', DEV.id),
        query: { confirm: 'GITHUB_CLIENT_ID' },
        idempotency_key: expect.any(String),
      },
    ])
  })

  it('Rotate credentials asks, then posts to the resource id from /resources', async () => {
    const h = await board('manage-settings', {}, { 'iii-cloud::api::write': () => ok({ id: 'rot-1' }) })
    await click(btn('Rotate credentials'))
    expect(writes(h.calls)).toEqual([])
    await dialogConfirm()
    await flush()
    expect(writes(h.calls)).toEqual([
      {
        method: 'POST',
        path: apiPath(`/resources/${DB_ID}/rotate-credentials`, DEV.id),
        idempotency_key: expect.any(String),
      },
    ])
  })

  it('a value changed after the serving deploy started offers to restart its groups', async () => {
    const h = await board('manage-settings', {}, { 'iii-cloud::api::write': () => ok() })
    expect(text()).toContain('STRIPE_SECRET_KEY changed after the running deploy started')
    expect(text()).toContain('app and jobs read it when they restart.')
    await click(btn('Restart app and jobs now'))
    await dialogConfirm()
    await flush()
    expect(writes(h.calls).map((w) => [w?.method, w?.path])).toEqual([
      ['POST', apiPath('/machines/groups/app/restart', DEV.id)],
      ['POST', apiPath('/machines/groups/jobs/restart', DEV.id)],
    ])
    expect(text()).not.toContain('changed after the running deploy started')
  })

  it('"Wait for the next deploy" dismisses the restart note without a call', async () => {
    const h = await board('manage-settings')
    const before = h.calls.length
    await click(btn('Wait for the next deploy'))
    expect(h.calls.length).toBe(before)
    expect(text()).not.toContain('changed after the running deploy started')
  })

  describe('secret values', () => {
    const only = secretWatch()

    it('Replace asks the worker for a request, then provides the value to it; never api::write', async () => {
      const h = await board('manage-settings', {}, { 'iii-cloud::secret': { id: 'sr-new' } })
      await click(el.querySelector('button[aria-label="Replace STRIPE_SECRET_KEY"]'))
      const value = 'sk_live_51Hx9-replaced'
      await type(el.querySelector('input[type="password"]'), value)
      await click(btn('Save'))
      await flush()
      const secret = h.calls.filter((c) => c.id.startsWith('iii-cloud::secret'))
      expect(secret).toEqual([
        { id: 'iii-cloud::secret', payload: { name: 'STRIPE_SECRET_KEY', description: 'Read by billing.' } },
        { id: 'iii-cloud::secret::provide', payload: { request_id: 'sr-new', value } },
      ])
      expect(writes(h.calls)).toEqual([])
      only(h.calls, value)
    })

    it('Add a secret: the name is constrained, the value goes only to secret::provide', async () => {
      const h = await board('manage-settings', {}, { 'iii-cloud::secret': { id: 'sr-add' } })
      await click(btn('Add'))
      await type(el.querySelector('[aria-label="Secrets and variables"] input'), 'new token-1')
      await click(btn('Secret'))
      expect(text()).toContain('NEWTOKEN1')
      const value = 'tok_9f8e7d6c5b4a'
      await type(el.querySelector('input[type="password"]'), value)
      await click(btn('Save'))
      await flush()
      expect(h.calls.filter((c) => c.id.startsWith('iii-cloud::secret')).map((c) => c.payload)).toEqual([
        { name: 'NEWTOKEN1', description: 'Added on the Settings page.' },
        { request_id: 'sr-add', value },
      ])
      only(h.calls, value)
    })
  })

  it('Add a variable writes it with PUT', async () => {
    const h = await board('manage-settings', {}, { 'iii-cloud::api::write': () => ok() })
    await click(btn('Add'))
    const inputs = () => el.querySelectorAll('[aria-label="Secrets and variables"] input')
    await type(inputs()[0], 'LOG_LEVEL')
    await type(inputs()[1], 'debug')
    await click(btn('Save'))
    await flush()
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

describe('change file (Mng11)', () => {
  it('lists the compose files, checks the chosen one and moves to the deploy check', async () => {
    const h = await board('manage-settings')
    await click(btn('Change…'))
    const radios = () => [...el.querySelectorAll('[role=dialog] [role=radio]')]
    expect(radios().map((r) => r.textContent)).toEqual([
      'api/worker-compose-prod.yamlwhat dev runs now · 11 workers',
      'api/worker-compose.staging.yaml11 workers',
      'api/workers-compose.yamlnot a worker-compose*.yaml',
    ])
    expect(radios()[2].getAttribute('aria-disabled')).toBe('true')
    expect(text()).toContain('The check plans this file and shows every difference before you deploy.')
    expect(btn('Check with this file')?.hasAttribute('disabled')).toBe(true)
    await click(radios()[1])
    expect(radios()[1].getAttribute('aria-checked')).toBe('true')
    await click(btn('Check with this file'))
    await flush()
    expect(h.calls.find((c) => c.id === 'iii-cloud::check')?.payload).toEqual({
      run: true,
      manifest: 'api/worker-compose.staging.yaml',
    })
    expect(el.querySelector('[role=dialog]')).toBeNull()
    expect(el.querySelector('[role=toolbar] nav')?.textContent).toBe('Projects / registry · dev / Deploy')
  })

  it('has no Change… when the worktree has no other file to pick', async () => {
    const s = boards['manage-settings']
    const one = { ...s.status, compose_files: s.status.compose_files?.slice(0, 1) }
    await mount(one, ctx(s.context))
    expect(btn('Change…')).toBeFalsy()
  })
})

describe("O4: the workers' config", () => {
  const panel = () => el.querySelector(`section[aria-label="Workers' config"]`)
  const reads = (h: { calls: Call[] }) =>
    h.calls.filter((c) => c.id === 'iii-cloud::api' && c.payload?.path === apiPath('/config', DEV.id))

  it('lists what iii cloud keeps per worker, read-only; Copy ./config again previews, applies, then reads it again', async () => {
    const h = await board('manage-settings-config')
    // Only what was set in iii cloud: a default field, or a worker with none set, stays out.
    const workers = [...(panel()?.querySelectorAll('.iii-cloud-ci-list > li') ?? [])]
    expect(workers.map((w) => w.querySelector('.iii-cloud-ci-head')?.textContent)).toEqual(['http', 'queue'])
    expect(workers[0].querySelector('pre')?.textContent).toBe('port = 3111\nhost = 0.0.0.0')
    expect(panel()?.querySelector('input, textarea')).toBeNull()
    expect(panel()?.querySelector('a')?.getAttribute('href')).toBe('https://cloud.iii.dev/iii/registry/dev/config')
    expect(reads(h)).toHaveLength(1)
    await click(btn('Copy ./config again'))
    expect(of(h, 'iii-cloud::config::import')).toEqual([{ environment_id: DEV.id, dry_run: true }])
    await click(btn("Copy 3 workers' config to dev"))
    await flush()
    expect(of(h, 'iii-cloud::config::import').at(-1)).toEqual({
      environment_id: DEV.id,
      dry_run: false,
      expected_version: 3,
    })
    expect(el.querySelector('[role=dialog]')).toBeNull()
    expect(reads(h)).toHaveLength(2)
  })

  it('nothing copied yet: says the workers run on their defaults, and offers the copy', async () => {
    await board('manage-settings-config', {}, cloudAnswers({ [apiPath('/config', DEV.id)]: { workers: [] } }))
    expect(panel()?.textContent).toContain(
      "Nothing copied yet: the workers start with their defaults on dev. ./config isn't in the repository.",
    )
    expect(btn('Copy ./config to iii cloud')).toBeTruthy()
  })

  it('no config folder and nothing set in iii cloud: no panel', async () => {
    await board('manage-settings')
    expect(panel()).toBeNull()
  })
})
