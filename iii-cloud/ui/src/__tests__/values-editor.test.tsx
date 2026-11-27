// The values editor (VALUES-PLAN §3.1): the names a compose file reads, where each value comes from, one Save
// through `values::save`. Typed values stay in the component: never in a payload but the save, storage or the page.
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
// The shared cases the worker's Rust parser reads too.
import envCases from '../../../tests/fixtures/env-parse.json'
import { rcFixing, TEST_ENV } from '../fixtures/check'
import { devValues, RC_PROD, RC_URL, rcSkippedValues, rcValues, valueItem } from '../fixtures/values'
import { Destinations } from '../SecretEntry'
import type { Status, Values } from '../types'
import { parseEnv, ValuesEditor, type ValuesEditorProps } from '../ValuesEditor'
import { type Call, fakeHost } from './host'
import { btn, byText, click, el, flush, root, text, type, ui, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

type Opts = Partial<Pick<ValuesEditorProps, 'mode' | 'narrow' | 'after' | 'from' | 'manifest'>>
/** The status with the values' environment selected (the page drops another environment's answer). */
const selecting = (v: { environment_id: string; environment: string }): Status => ({
  ...rcFixing,
  project: {
    ...rcFixing.project!,
    environment: { ...rcFixing.project!.environment!, id: v.environment_id, name: v.environment },
  },
})
async function editor(values: Values, answers: Parameters<typeof fakeHost>[1] = {}, opts: Opts = {}, given?: Status) {
  const s = given ?? selecting(values)
  const h = fakeHost(s, { 'iii-cloud::values': values, ...answers })
  const onSaved = vi.fn()
  const onSkip = vi.fn()
  await act(async () =>
    root.render(
      <ValuesEditor
        host={h.host}
        status={s}
        mode={opts.mode ?? 'dialog'}
        narrow={opts.narrow ?? false}
        after={opts.after}
        from={opts.from}
        manifest={opts.manifest}
        onSaved={onSaved}
        onSkip={onSkip}
      />,
    ),
  )
  await flush()
  return { ...h, onSaved, onSkip }
}

const of = (calls: Call[], id: string) => calls.filter((c) => c.id === id).map((c) => c.payload)
const names = () => [...el.querySelectorAll('.iii-cloud-val-row')].map((r) => r.getAttribute('data-name'))
const rowOf = (name: string) => el.querySelector(`.iii-cloud-val-row[data-name="${name}"]`)
const select = (name: string) => el.querySelector(`select[aria-label="Where ${name} comes from"]`) as HTMLSelectElement
const input = (name: string) => el.querySelector(`[aria-label="Value of ${name}"]`) as HTMLInputElement
async function pick(name: string, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value')?.set?.call(select(name), value)
    select(name).dispatchEvent(new Event('change', { bubbles: true }))
  })
}
async function typeArea(area: Element | null, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set?.call(area, value)
    area?.dispatchEvent(new Event('input', { bubbles: true }))
  })
}
const DATABASE_REF = '$' + '{DATABASE_PRIMARY_URL}'
const tab = (label: string) => [...el.querySelectorAll('[role=tab]')].find((t) => t.textContent === label)

describe('parseEnv', () => {
  it.each(envCases as { text: string; expected: { name: string; value: string }[] }[])('reads the shared case %#', ({
    text,
    expected,
  }) => {
    expect(parseEnv(text)).toEqual(expected)
  })
})

describe('the list', () => {
  it('several containers: a group each; a name read by two appears once, "also read by"; Not set first', async () => {
    const h = await editor(devValues, {}, { from: 'checkout', manifest: 'api/worker-compose-prod.yaml' })
    expect(of(h.calls, 'iii-cloud::values')).toEqual([{ manifest: 'api/worker-compose-prod.yaml', from: 'checkout' }])
    expect(tab('Not set 3')?.getAttribute('aria-selected')).toBe('true')
    expect(tab('All 4')).toBeTruthy()
    expect([...el.querySelectorAll('.iii-cloud-val-ghead')].map((g) => g.textContent)).toEqual([
      'billing · 1',
      'search · 2',
    ])
    expect(names()).toEqual(['STRIPE_SECRET_KEY', 'SEARCH_WEIGHT_STARS', 'SENTRY_DSN'])
    expect(rowOf('SENTRY_DSN')?.textContent).toContain('Also read by billing.')
    await click(tab('All 4'))
    expect(names()).toEqual(['STRIPE_SECRET_KEY', 'SEARCH_WEIGHT_STARS', 'SENTRY_DSN', 'GITHUB_CLIENT_ID'])
    // A set variable shows its value and keeps it by default; a set name offers Replace, never a preselection.
    expect(rowOf('GITHUB_CLIENT_ID')?.textContent).toContain('Iv1.8a61f9b3a8')
    expect(rowOf('GITHUB_CLIENT_ID')?.textContent).toMatch(/Set in iii cloud · 13h ago/)
    expect(select('GITHUB_CLIENT_ID').value).toBe('keep')
    expect([...select('GITHUB_CLIENT_ID').options].map((o) => o.textContent)).toEqual([
      'Keep',
      'Replace',
      'Leave empty',
    ])
  })

  it('one container: no group header; 22 names, 21 under Not set; a default only under All', async () => {
    await editor(rcValues)
    expect(el.querySelector('.iii-cloud-val-ghead')).toBeNull()
    expect(text()).toContain("api/worker-compose-prod.yaml reads 22 names; 21 aren't set.")
    expect(names()).toHaveLength(21)
    expect(names()).not.toContain('RELEASE_SHA')
    await click(tab('All 22'))
    expect(names()).toHaveLength(22)
    expect(rowOf('RELEASE_SHA')?.textContent).toContain('default: development')
    expect(rowOf('SESSION_SECRET')?.textContent).toContain('Asked by your agent')
    expect(rowOf('SESSION_SECRET')?.textContent).toContain('Signs the session cookies of the web app.')
  })

  it('preselects: the address for APP_BASE_URL, .env.test for GH_*, never .env.production; localhost says so', async () => {
    await editor(rcValues)
    expect(select('APP_BASE_URL').value).toBe('env_url')
    expect(rowOf('APP_BASE_URL')?.textContent).toContain(RC_URL)
    expect(select('GH_ORG').value).toBe('env_file:.env.test')
    expect([...select('GH_ORG').options].map((o) => o.textContent)).toContain('From .env.production')
    expect(select('RELEASE_NOTES_URL').value).toBe('typed')
    expect([...select('RELEASE_NOTES_URL').options].map((o) => o.textContent)).toContain(
      'From .env.local (points at localhost)',
    )
    expect([...select('SLACK_CHANNEL').options].map((o) => o.textContent)).toContain('Copy from production')
  })

  it('a secret is typed in a password field, its Multi-line text area masked until Show; Variable shows it', async () => {
    await editor(rcValues)
    expect(input('SESSION_SECRET').type).toBe('password')
    expect(input('SESSION_SECRET').placeholder).toBe('Paste the secret')
    const row = '.iii-cloud-val-row[data-name="SESSION_SECRET"] button'
    await click(byText(row, 'Multi-line'))
    expect(input('SESSION_SECRET').tagName).toBe('TEXTAREA')
    // Masked by default; Show is the person's, and Hide masks it again.
    expect(input('SESSION_SECRET').hasAttribute('data-masked')).toBe(true)
    expect(byText(row, 'Show')?.getAttribute('aria-pressed')).toBe('false')
    await click(byText(row, 'Show'))
    expect(input('SESSION_SECRET').hasAttribute('data-masked')).toBe(false)
    await click(byText(row, 'Hide'))
    expect(input('SESSION_SECRET').hasAttribute('data-masked')).toBe(true)
    await click(byText(row, 'One line'))
    const kind = [...(rowOf('SESSION_SECRET')?.querySelectorAll('[role=tab]') ?? [])]
    await click(kind.find((t) => t.textContent === 'Variable'))
    expect(input('SESSION_SECRET').type).toBe('text')
    // A variable is one line: no Multi-line.
    expect(byText('.iii-cloud-val-row[data-name="SESSION_SECRET"] button', 'Multi-line')).toBeUndefined()
  })

  it('a database row has no field: iii cloud makes it, unless the person says it is outside', async () => {
    await editor(rcValues)
    expect(rowOf('DATABASE_URL')?.querySelector('input, select, textarea')).toBeNull()
    expect(rowOf('DATABASE_URL')?.textContent).toContain(
      `iii cloud makes this database. Your agent points DATABASE_URL at ${DATABASE_REF}.`,
    )
    await click(byText('.iii-cloud-val-row[data-name="DATABASE_URL"] button', "It's outside iii cloud"))
    expect(input('DATABASE_URL').type).toBe('password')
  })

  it('on a phone the editor says so (each row a card)', async () => {
    ui.narrow = true
    await editor(rcValues, {}, { narrow: true, mode: 'step' })
    expect(el.querySelector('.iii-cloud-val')?.hasAttribute('data-narrow')).toBe(true)
    expect(names()).toHaveLength(21)
  })

  it('a long name wraps only after its underscores; the kind toggle is not on the name’s line', async () => {
    await editor(rcValues, {}, { narrow: true })
    const key = rowOf('HARNESS_E2E_OIDC_AUDIENCE')?.querySelector('.iii-cloud-val-key')
    expect(key?.textContent).toBe('HARNESS_E2E_OIDC_AUDIENCE')
    expect(key?.innerHTML).toContain('HARNESS_<wbr>E2E_<wbr>OIDC_<wbr>AUDIENCE')
    // The name and its tags are one part; the kind is its own (on a phone, beside the tags).
    const name = rowOf('HARNESS_E2E_OIDC_AUDIENCE')?.querySelector('.iii-cloud-val-name')
    expect(name?.querySelector('.iii-cloud-val-tags')?.textContent).toBe('optionalNot set')
    expect(name?.querySelector('[aria-label="Kind of HARNESS_E2E_OIDC_AUDIENCE"]')).toBeNull()
  })
})

describe('Save', () => {
  it('counts what is chosen, sums it up, and nothing is written before Save', async () => {
    const h = await editor(rcValues, {}, { after: 'and check again' })
    expect(btn('Save 12 values and check again')).toBeTruthy()
    expect(text()).toContain('11 from .env.test · 1 address')
    await type(input('SESSION_SECRET'), 'typed-1')
    await pick('LOG_LEVEL', 'leave_empty')
    await pick('SLACK_CHANNEL', 'copy:8d1f4b6a-2c3e-4f5a-9b8c-7d6e5f4a3b2c')
    expect(btn('Save 15 values and check again')).toBeTruthy()
    expect(text()).toContain('11 from .env.test · 1 typed · 1 address · 1 copied · 1 left empty')
    expect(of(h.calls, 'iii-cloud::values::save')).toEqual([])
  })

  it('the payload: env_file sends its file and no value; typed sends its value; each source its own shape', async () => {
    const results = (p: Record<string, unknown>) => ({
      environment_id: rcValues.environment_id,
      results: (p.items as { name: string }[]).map((x) => ({ name: x.name, ok: true })),
    })
    const h = await editor(rcValues, { 'iii-cloud::values::save': results })
    await type(input('SESSION_SECRET'), 'typed-1')
    await pick('LOG_LEVEL', 'leave_empty')
    await click(btn('Save 14 values'))
    await flush()
    const [sent] = of(h.calls, 'iii-cloud::values::save') as {
      environment_id: string
      manifest: string
      items: Record<string, unknown>[]
    }[]
    expect(sent.environment_id).toBe(rcValues.environment_id)
    expect(sent.manifest).toBe('api/worker-compose-prod.yaml')
    const by = Object.fromEntries(sent.items.map((x) => [x.name, x]))
    expect(by.APP_BASE_URL).toEqual({ name: 'APP_BASE_URL', from: 'env_url' })
    expect(by.GH_ORG).toEqual({ name: 'GH_ORG', from: 'env_file', type: 'secret', file: '.env.test' })
    expect(by.SESSION_SECRET).toEqual({ name: 'SESSION_SECRET', from: 'typed', type: 'secret', value: 'typed-1' })
    expect(by.LOG_LEVEL).toEqual({ name: 'LOG_LEVEL', from: 'leave_empty' })
    expect(sent.items.filter((x) => x.from === 'env_file').every((x) => !('value' in x))).toBe(true)
    expect(by.DATABASE_URL).toBeUndefined()
    expect(h.onSaved).toHaveBeenCalledTimes(1)
    // Saved: each row says so, its value is gone, and the list is read again.
    expect(rowOf('SESSION_SECRET')?.textContent).toContain('Saved')
    expect(el.innerHTML).not.toContain('typed-1')
    expect(of(h.calls, 'iii-cloud::values')).toHaveLength(2)
  })

  it('per row: a refused one says why in plain words and keeps its choice; no onSaved', async () => {
    const answer = {
      environment_id: rcValues.environment_id,
      results: [
        { name: 'SESSION_SECRET', ok: false, error: { code: 'NOT_TRIED', message: 'not tried' } },
        { name: 'SLACK_CHANNEL', ok: false, error: { code: 'ALREADY_SET', message: 'skipped' } },
      ],
    }
    const only = {
      ...rcValues,
      items: rcValues.items.filter((i) => ['SESSION_SECRET', 'SLACK_CHANNEL'].includes(i.name)),
    }
    const h = await editor(only, { 'iii-cloud::values::save': answer })
    await type(input('SESSION_SECRET'), 'kept-value')
    await pick('SLACK_CHANNEL', 'copy:8d1f4b6a-2c3e-4f5a-9b8c-7d6e5f4a3b2c')
    await click(btn('Save 2 values'))
    await flush()
    expect(rowOf('SESSION_SECRET')?.querySelector('[role=alert]')?.textContent).toBe(
      'Not saved. The save stopped before this one. It stays chosen for the next Save.',
    )
    expect(rowOf('SLACK_CHANNEL')?.querySelector('[role=alert]')?.textContent).toBe(
      "Not saved. It's already set in test; a copy never replaces a value. It stays chosen for the next Save.",
    )
    expect(input('SESSION_SECRET').value).toBe('kept-value')
    expect(btn('Save 2 values')).toBeTruthy()
    expect(h.onSaved).not.toHaveBeenCalled()
  })

  it("saved in iii cloud but not in the run's .env (LOCAL_NOT_SAVED): a faint note, not an error", async () => {
    const only = { ...rcValues, items: rcValues.items.filter((i) => i.name === 'SESSION_SECRET') }
    const answer = {
      environment_id: rcValues.environment_id,
      results: [{ name: 'SESSION_SECRET', ok: true, error: { code: 'LOCAL_NOT_SAVED', message: 'tracked' } }],
    }
    const h = await editor(only, { 'iii-cloud::values::save': answer })
    await type(input('SESSION_SECRET'), 'a-value')
    await click(btn('Save 1 value'))
    await flush()
    expect(rowOf('SESSION_SECRET')?.querySelector('[role=alert]')).toBeNull()
    expect(rowOf('SESSION_SECRET')?.textContent).toContain('Saved · just now')
    expect(rowOf('SESSION_SECRET')?.textContent).toContain(
      "Saved in iii cloud. The .env of the local run couldn't be written, so the local run doesn't have it.",
    )
    expect(h.onSaved).toHaveBeenCalledTimes(1)
  })

  it("NOT_SAVED keeps the worker's own words; FILE_CHANGED names the file and the name", async () => {
    const answer = {
      environment_id: rcValues.environment_id,
      results: [
        { name: 'GH_ORG', ok: false, error: { code: 'FILE_CHANGED', message: 'gone' } },
        {
          name: 'SESSION_SECRET',
          ok: false,
          error: { code: 'NOT_SAVED', message: 'iii cloud refused it (VALUE_TOO_LONG).' },
        },
      ],
    }
    const only = { ...rcValues, items: rcValues.items.filter((i) => ['GH_ORG', 'SESSION_SECRET'].includes(i.name)) }
    await editor(only, { 'iii-cloud::values::save': answer })
    await type(input('SESSION_SECRET'), 'x')
    await click(btn('Save 2 values'))
    await flush()
    expect(rowOf('GH_ORG')?.querySelector('[role=alert]')?.textContent).toBe(
      'Not saved. .env.test no longer has GH_ORG. Pick another source, or add it back to the file. It stays chosen for the next Save.',
    )
    expect(rowOf('SESSION_SECRET')?.querySelector('[role=alert]')?.textContent).toBe(
      'Not saved. iii cloud refused it (VALUE_TOO_LONG). It stays chosen for the next Save.',
    )
  })

  it('a refused call (ENVIRONMENT_CHANGED) shows the error and keeps every choice', async () => {
    const err = Object.assign(new Error('The selected environment changed.'), { code: 'ENVIRONMENT_CHANGED' })
    await editor(rcValues, { 'iii-cloud::values::save': err })
    await type(input('SESSION_SECRET'), 'kept-value')
    await click(btn('Save 13 values'))
    await flush()
    expect(text()).toContain('Another environment was selected while you chose. Nothing was saved')
    expect(input('SESSION_SECRET').value).toBe('kept-value')
    expect(btn('Save 13 values')).toBeTruthy()
  })

  it('a typed value never reaches storage, pane state or any other call', async () => {
    // jsdom's storages (Node's own global localStorage shadows them without --localstorage-file).
    const w = (globalThis as { jsdom?: { window: Window } }).jsdom?.window ?? window
    const [localStorage, sessionStorage] = [w.localStorage, w.sessionStorage]
    localStorage.clear()
    sessionStorage.clear()
    const saved = { environment_id: rcValues.environment_id, results: [] }
    const h = await editor(rcValues, { 'iii-cloud::values::save': saved })
    await type(input('SESSION_SECRET'), 'never-stored-9f2')
    expect(localStorage.length).toBe(0)
    expect(sessionStorage.length).toBe(0)
    expect(JSON.stringify(ui.panes)).not.toContain('never-stored-9f2')
    await click(btn('Save 13 values'))
    await flush()
    expect(h.calls.filter((c) => JSON.stringify(c).includes('never-stored-9f2')).map((c) => c.id)).toEqual([
      'iii-cloud::values::save',
    ])
    expect(localStorage.length + sessionStorage.length).toBe(0)
  })
})

describe('Paste a .env', () => {
  it('fills the listed names as typed values, reports the others by name, and clears', async () => {
    const h = await editor(rcValues)
    await click(btn('Paste a .env'))
    const area = el.querySelector('.iii-cloud-val-paste textarea')
    await typeArea(area, 'GH_ORG=iii-hq\nFOO=1\nexport SESSION_SECRET="s3cret"\nBAR=2\nDATABASE_URL=postgres://x\n')
    await click(btn('Apply'))
    expect(el.querySelector('.iii-cloud-val-paste')).toBeNull()
    expect(el.querySelector('[role=status]')?.textContent).toBe(
      'Filled 2 of 22. Not read by api/worker-compose-prod.yaml: FOO, BAR.',
    )
    expect(text()).toContain('10 from .env.test · 2 typed · 1 address · the pasted text is gone')
    expect(select('GH_ORG').value).toBe('typed')
    expect(input('GH_ORG').value).toBe('iii-hq')
    expect(input('SESSION_SECRET').value).toBe('s3cret')
    // The database row stays iii cloud's.
    expect(rowOf('DATABASE_URL')?.querySelector('input')).toBeNull()
    expect(text()).not.toContain('postgres://x')
    expect(of(h.calls, 'iii-cloud::values::save')).toEqual([])
  })
})

describe('Destinations', () => {
  it("an API key points at the environment's config page, not at a variables page", async () => {
    const s = { ...rcFixing, connection: { ...rcFixing.connection, method: 'api_key' as const } }
    await act(async () => root.render(<Destinations status={s} saved={false} />))
    expect(byText('a', "Set it in iii cloud's console too")?.getAttribute('href')).toMatch(/\/test\/config$/)
  })
})

describe('read-only', () => {
  it('an API key: the list without pickers, the console link and the browser sign-in; Check still works', async () => {
    const h = await editor({ ...rcValues, save: 'api_key' }, {}, { mode: 'step' }, { ...rcFixing })
    expect(text()).toContain(
      "An API key can't write secrets and variables. Set them in iii cloud's console, or sign in with the browser.",
    )
    const link = byText('a', "Open test's values in iii cloud")
    expect(link?.getAttribute('href')).toMatch(/\/test\/config$/)
    expect(text()).toContain("Continue still works: the check lists what's missing.")
    expect(el.querySelector('select, input, textarea')).toBeNull()
    expect(btn('Paste a .env')).toBeNull()
    await click(btn('Sign in with your browser'))
    expect(of(h.calls, 'iii-cloud::connect')).toEqual([{ method: 'browser' }])
    await click(btn('Check'))
    expect(h.onSkip).toHaveBeenCalledTimes(1)
  })

  it('production without Owner or Admin: read-only, and says who can change it', async () => {
    await editor(
      { ...rcValues, environment_id: RC_PROD.id, environment: RC_PROD.name, save: 'role' },
      {},
      {
        mode: 'dialog',
      },
    )
    expect(text()).toContain("Changing production's secrets and variables takes Owner or Admin.")
    expect(text()).toContain('Ask an Owner or Admin, or continue to the check.')
    expect(el.querySelector('select, input, textarea')).toBeNull()
    expect(btn('Close')).toBeTruthy()
  })
})

describe('the step', () => {
  it('P1: its title and forward action are its own header; the footer keeps only the summary', async () => {
    await editor(rcValues, {}, { mode: 'step' })
    expect(el.querySelector('.iii-cloud-stephead h2')?.textContent).toBe(
      'Give the secrets and variables your app reads',
    )
    expect(el.querySelector('.iii-cloud-stephead-fwd button')?.textContent).toBe('Save 12 values and check')
    expect(el.querySelector('.iii-cloud-stephead-back button')?.textContent).toBe('Check without saving')
    expect(el.querySelector('.iii-cloud-val-foot button')).toBeNull()
  })

  it('P3: what the local run lacks is Not set, "needed for the local run"; a from_file one says its file; never the database', async () => {
    const v = {
      ...rcValues,
      items: [
        valueItem('SESSION_SECRET', { need: 'required', default: null, in_cloud: 'set', here: 'unset' }),
        valueItem('GH_ORG', { here: 'from_file', here_file: 'api/.env.test' }),
        valueItem('DATABASE_URL', { database: true, in_cloud: 'set', here: 'unset' }),
      ],
    }
    await editor(v, {}, { mode: 'step' })
    // Set in iii cloud but not here: still to do, so it is listed and counted.
    expect(names()).toEqual(['SESSION_SECRET', 'GH_ORG'])
    expect(rowOf('SESSION_SECRET')?.querySelector('.iii-cloud-val-state')?.textContent).toBe('Not set')
    expect(rowOf('SESSION_SECRET')?.textContent).toContain('needed for the local run')
    // Keeping iii cloud's value would leave the local run without it: the field waits for one.
    expect(input('SESSION_SECRET')).toBeTruthy()
    expect(rowOf('GH_ORG')?.textContent).toContain('the local run takes it from api/.env.test')
    expect(rowOf('GH_ORG')?.textContent).not.toContain('needed for the local run')
  })

  it('Save N values and check, or Check without saving; with nothing unset, all set and Show all', async () => {
    await editor(rcValues, {}, { mode: 'step' })
    expect(btn('Save 12 values and check')).toBeTruthy()
    expect(btn('Check without saving')).toBeTruthy()
    const set = { ...devValues, items: devValues.items.map((i) => ({ ...i, in_cloud: 'set' as const })) }
    await editor(set, {}, { mode: 'step' })
    expect(text()).toContain('All 4 values api/worker-compose-prod.yaml reads are set in dev.')
    expect(names()).toEqual([])
    expect(text()).toContain('Nothing to save. The check reads these.')
    await click(btn('Show all 4'))
    expect(names()).toHaveLength(4)
    expect(btn('Check')).toBeTruthy()
    expect(btn('Check without saving')).toBeNull()
  })

  it("a name iii cloud can't store: no source, no field, the reason faint; never saved", async () => {
    const odd = {
      ...rcValues,
      items: [
        valueItem('lower_case', { invalid_name: 'iii cloud stores upper-case names only.' }),
        valueItem('SESSION_SECRET'),
      ],
    }
    const h = await editor(odd, { 'iii-cloud::values::save': { environment_id: TEST_ENV.id, results: [] } })
    expect(select('lower_case')).toBeNull()
    expect(input('lower_case')).toBeNull()
    expect(rowOf('lower_case')?.querySelector('.iii-cloud-val-note')?.textContent).toBe(
      'iii cloud stores upper-case names only.',
    )
    await type(input('SESSION_SECRET'), 'x')
    await click(btn('Save 1 value'))
    expect(of(h.calls, 'iii-cloud::values::save')[0]).toMatchObject({
      items: [{ name: 'SESSION_SECRET', from: 'typed' }],
    })
  })

  it('a value typed in test never follows a switch to production: the field is empty, Save leaves it out', async () => {
    const test = selecting(rcValues)
    const prod = selecting({ environment_id: RC_PROD.id, environment: RC_PROD.name })
    const prodValues = { ...rcValues, environment_id: RC_PROD.id, environment: RC_PROD.name }
    let now: Values = rcValues
    const h = fakeHost(test, {
      'iii-cloud::values': () => now,
      'iii-cloud::values::save': { environment_id: RC_PROD.id, results: [] },
    })
    const show = (s: Status) =>
      act(async () => root.render(<ValuesEditor host={h.host} status={s} mode="dialog" narrow={false} />))
    await show(test)
    await flush()
    await type(input('SESSION_SECRET'), 'test-only')
    expect(input('SESSION_SECRET').value).toBe('test-only')
    now = prodValues
    await show(prod)
    await flush()
    expect(input('SESSION_SECRET').value).toBe('')
    await click(btn('Save 12 values'))
    const sent = of(h.calls, 'iii-cloud::values::save')[0] as { environment_id: string; items: { name: string }[] }
    expect(sent.environment_id).toBe(RC_PROD.id)
    expect(sent.items.map((x) => x.name)).not.toContain('SESSION_SECRET')
    expect(JSON.stringify(h.calls)).not.toContain('test-only')
  })

  it("an answer for another environment than the selected one is dropped: test's names never show under production", async () => {
    await editor(rcValues, {}, { mode: 'dialog' }, selecting({ environment_id: RC_PROD.id, environment: 'production' }))
    expect(names()).toEqual([])
  })

  it('a name the file reads with no answer yet is a row with Type a value', async () => {
    const one = { ...rcValues, items: [valueItem('NEW_ONE', { need: 'required', default: null })] }
    await editor(one, {}, { mode: 'step' })
    expect(select('NEW_ONE').value).toBe('typed')
    expect([...select('NEW_ONE').options].map((o) => o.textContent)).toEqual(['Type a value', 'Skip for now'])
  })
})

describe('SKIP-VALUES', () => {
  const skipAnswer = (p: Record<string, unknown>) => ({ environment_id: TEST_ENV.id, names: p.names })
  const saved = (p: Record<string, unknown>) => ({
    environment_id: TEST_ENV.id,
    results: (p.items as { name: string }[]).map((x) => ({ name: x.name, ok: true })),
  })

  it('"Skip for now" in the picker; "Skip the rest" picks it on every unchosen Not set row; Save, then skip, in order', async () => {
    const h = await editor(
      rcValues,
      { 'iii-cloud::values::save': saved, 'iii-cloud::values::skip': skipAnswer },
      { mode: 'step' },
    )
    expect([...select('LOG_LEVEL').options].map((o) => o.textContent)).toContain('Skip for now')
    expect(btn('Save 12 values and check')).toBeTruthy()
    await click(btn('Skip the rest'))
    // 21 not set: 12 chosen, the database row has no picker, SESSION_SECRET and 7 more get skipped.
    expect(select('LOG_LEVEL').value).toBe('skip')
    expect(rowOf('LOG_LEVEL')?.textContent).toContain('skipped when you save')
    expect(btn('Skip the rest')).toBeNull()
    const label = el.querySelector('.iii-cloud-stephead-fwd button')?.textContent ?? ''
    expect(label).toMatch(/^Save 12 · skip \d+ and check$/)
    const k = Number(label.match(/skip (\d+)/)?.[1])
    expect(el.querySelector('.iii-cloud-val-sum')?.textContent).toContain(`· ${k} skipped`)
    await click(el.querySelector('.iii-cloud-stephead-fwd button'))
    await flush()
    const ids = h.calls.map((c) => c.id).filter((id) => id.startsWith('iii-cloud::values::'))
    expect(ids).toEqual(['iii-cloud::values::save', 'iii-cloud::values::skip'])
    const skip = of(h.calls, 'iii-cloud::values::skip')[0] as { names: string[]; skip: boolean }
    expect(skip.skip).toBe(true)
    expect(skip.names).toHaveLength(k)
    expect(skip.names).toContain('LOG_LEVEL')
    expect(skip.names).not.toContain('DATABASE_URL')
    expect(h.onSaved).toHaveBeenCalledTimes(1)
  })

  it('only skips: "Skip N and check", no save call', async () => {
    const one = { ...rcValues, items: [valueItem('LOG_LEVEL'), valueItem('SENTRY_DSN')] }
    const h = await editor(one, { 'iii-cloud::values::skip': skipAnswer }, { mode: 'step' })
    await click(btn('Skip the rest'))
    await click(btn('Skip 2 and check'))
    await flush()
    expect(of(h.calls, 'iii-cloud::values::save')).toEqual([])
    expect(of(h.calls, 'iii-cloud::values::skip')).toEqual([
      { environment_id: TEST_ENV.id, names: ['LOG_LEVEL', 'SENTRY_DSN'], skip: true },
    ])
    expect(h.onSaved).toHaveBeenCalledTimes(1)
  })

  it('a skipped row: the chip "Skipped" with Undo at its right; it is not sent again', async () => {
    const h = await editor(rcSkippedValues, { 'iii-cloud::values::skip': skipAnswer }, { mode: 'step' })
    const chip = rowOf('LOG_LEVEL')?.querySelector('.iii-cloud-val-state')
    expect(chip?.textContent).toBe('Skipped')
    expect(chip?.nextElementSibling?.textContent).toBe('Undo')
    expect(select('LOG_LEVEL').value).toBe('skip')
    expect(btn('Save 12 values and check')).toBeTruthy()
    await click(chip?.nextElementSibling ?? null)
    expect(of(h.calls, 'iii-cloud::values::skip')).toEqual([
      { environment_id: TEST_ENV.id, names: ['LOG_LEVEL'], skip: false },
    ])
  })
})
