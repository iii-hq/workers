// The Deploys tab (U7): the list beside one deploy (Mng2), its failure, its log, rollback (Mng7), the phone (Mng10).
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { CloudPage } from '../CloudPage'
import { boards } from '../fixtures'
import { apiPath, base, earlierReady, failedDeploy, following, pr, ready, T } from '../fixtures/base'
import { DEV, devList } from '../fixtures/deploys'
import type { Status } from '../types'
import { fakeHost } from './host'
import {
  board,
  btn,
  byText,
  click,
  cloudAnswers,
  ctx,
  el,
  flush,
  lastCall,
  mount,
  root,
  text,
  type,
  ui,
  usePage,
} from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

const at = (s: string, within = text()) => within.indexOf(s)
const logText = () => el.querySelector('section[aria-label=Log]')?.textContent ?? ''
const gets = (calls: { id: string; payload?: Record<string, unknown> }[], path: string) =>
  calls.filter((c) => c.id === 'iii-cloud::api' && c.payload?.path === path)
const listWords = () =>
  [...el.querySelectorAll('[aria-label=Deploys] [role=row]')].map((r) => r.querySelector('[data-chip]')?.textContent)

describe('Mng2: the list beside one deploy', () => {
  it('lists every deploy with its number, word, commit and time; the routed one is open', async () => {
    await board('manage-deploys-failed')
    expect(listWords()).toEqual(['Live', 'Superseded', 'Superseded', 'Superseded', 'Failed', 'Failed'])
    const rows = [...el.querySelectorAll('[aria-label=Deploys] [role=row]')]
    expect(rows[0].textContent).toContain('#6')
    expect(rows[0].textContent).toContain('Search: rank by stars')
    expect(rows[0].textContent).toContain('9c04e7f')
    expect(el.querySelector('[aria-label=Deploys] [aria-current=true]')?.textContent).toContain('#2')
    expect(text()).toContain('Deploy #2 · 86c49e1 · ')
    expect(text()).toContain('from a push to GitHub, not checked here first')
    const file = byText('a', 'see the file in 86c49e1')
    expect(file?.getAttribute('href')).toBe(
      'https://github.com/iii-hq/registry/blob/86c49e1d2a3b4c5d6e7f8091a2b3c4d5e6f70812/api/worker-compose-prod.yaml',
    )
    expect(text()).toContain('Current app file api/worker-compose-prod.yaml')
  })

  it('the plain title comes before the fix and the log; nothing was live, so nobody saw it', async () => {
    await board('manage-deploys-failed')
    const t = text()
    const title = 'Your app starts its own web server; iii cloud only sends traffic to its http package'
    expect(at(title)).toBeGreaterThanOrEqual(0)
    expect(at(title)).toBeLessThan(at('Fix with your agent'))
    expect(at('Fix with your agent')).toBeLessThan(at('first unserved route'))
    expect(t).toContain('Nothing was live on dev then, so nobody saw an error.')
    expect(t).toContain('4 of 6 passed · Workers failed')
    expect(t).toContain('not reached')
  })

  it('reads the log from iii cloud: the failing stage first, oldest line first, then by tab and errors only', async () => {
    const h = await board('manage-deploys-failed')
    const logs = apiPath('/logs', DEV.id)
    expect(gets(h.calls, logs).at(-1)?.payload).toEqual({
      method: 'GET',
      path: logs,
      query: { deployment_id: 'dep-2', source: 'worker' },
    })
    expect(at('started in group app', logText())).toBeLessThan(at('code ROUTE_OUTSIDE_HTTP_GROUP', logText()))
    await click(btn('Release'))
    expect(gets(h.calls, logs).at(-1)?.payload?.query).toEqual({ deployment_id: 'dep-2', source: 'release' })
    // iii cloud has no release lines for #2, and the worker's lines are the workers': none stand in.
    expect(logText()).toContain('No release command lines for this deploy')
    await click(el.querySelector('section[aria-label=Log] input[type=checkbox]'))
    expect(gets(h.calls, logs).at(-1)?.payload?.query).toEqual({
      deployment_id: 'dep-2',
      source: 'release',
      level: 'error',
    })
  })

  it('a running deploy reads its log again every 5 s while the page is visible', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] })
    try {
      const s = boards['manage-deploys-failed'].status
      const running = { ...s.deploys[0], id: 'dep-7', number: 7, status: 'in_progress', outcome: 'running' } as const
      const h = await mount(
        { ...s, deploys: [{ ...running, finished_at: null }, ...s.deploys] },
        ctx({ at: 'env', env: DEV.id, tab: 'deploys', deploy: 'dep-7' }),
      )
      const before = gets(h.calls, apiPath('/logs', DEV.id)).length
      expect(text()).toContain('follows the deploy, read every 5 s')
      await act(async () => void vi.advanceTimersByTime(5_000))
      await flush()
      expect(gets(h.calls, apiPath('/logs', DEV.id)).length).toBe(before + 1)
    } finally {
      vi.useRealTimers()
    }
  })

  it('a row opens its deploy; the deploy a card names opens directly', async () => {
    await board('manage-deploys-failed')
    await click([...el.querySelectorAll('[aria-label=Deploys] [role=row]')][0])
    expect(text()).toContain('Deploy #6 · 9c04e7f · live since')
    await board('manage-deploys-failed', ctx({ at: 'env', env: DEV.id, tab: 'deploys', deploy: 'dep-5' }, 2))
    expect(el.querySelector('[aria-label=Deploys] [aria-current=true]')?.textContent).toContain('#5')
    expect(text()).toContain('Deploy #5 · d738d2f · live ')
    expect(text()).toContain(' to ')
    expect(text()).toContain('by you')
  })

  it('a deploy that is not among the latest 20 says so', async () => {
    await mount(boards['manage-deploys-failed'].status, ctx({ at: 'env', env: DEV.id, tab: 'deploys', deploy: 'nope' }))
    expect(text()).toContain('Deploy not found')
    expect(text()).toContain('It is not among the latest 20 deploys of dev.')
  })
})

describe('Mng10: a failure on a phone', () => {
  it('one pane: back, the title, the fix, the last lines folded open; Back returns to the list', async () => {
    ui.narrow = true
    await board('manage-deploys-phone')
    expect(el.querySelector('[aria-label=Deploys]')).toBeNull()
    expect(text()).toContain('dev · deploy #2')
    expect(text()).toContain('Your agent works on your computer; you can follow it from here.')
    expect(text()).toContain('Last lines of the log')
    expect(text()).toContain('group app has no http package: routes are not served')
    expect(el.querySelector('section[aria-label=Log]')).toBeNull()
    await click(btn('Back to deploys'))
    expect(el.querySelector('[aria-label=Deploys]')).not.toBeNull()
    expect(text()).not.toContain('dev · deploy #2')
  })
})

describe('Mng7: rollback', () => {
  const rollbackPath = apiPath('/deployments/dep-5/rollback', DEV.id)
  const listPath = apiPath('/deployments', DEV.id)
  const newer = {
    deployments: [
      { ...devList.deployments[0], id: 'dep-7', number: 7, current: false, created_at: new Date().toISOString() },
      ...devList.deployments,
    ],
    next_cursor: null,
  }
  /** The rollback board's answers, with `/deployments` from `list()` on every read and the write's answer. */
  const answers = (list: () => unknown, write: (p: Record<string, unknown>) => unknown) => {
    const base = cloudAnswers(boards['manage-rollback'].cloud ?? {})
    return {
      'iii-cloud::api': (p: Record<string, unknown>) =>
        p.path === listPath ? { status: 200, body: list() } : base['iii-cloud::api'](p),
      'iii-cloud::api::write': write,
    }
  }

  it('the dialog is iii cloud preview of what goes back and what stays', async () => {
    const h = await board('manage-rollback')
    expect(text()).toContain('Drop the separate web container')
    expect(gets(h.calls, rollbackPath)).toHaveLength(0)
    await click(btn('Roll back dev to #5'))
    expect(gets(h.calls, rollbackPath)).toHaveLength(1)
    const d = el.querySelector('[role=dialog]')?.textContent ?? ''
    expect(d).toContain('Roll back dev to #5?')
    expect(d).toContain('dev goes back to d738d2f from 9c04e7f. Your branch keeps the newer commits')
    expect(d).toContain('code, worker images, config version 4')
    expect(d).toContain('data, secrets, resources created later')
    expect(d).toContain('Migrations run by #6 are not reverted: 0008_trgm.sql stays.')
    expect(d).toContain('Nothing is queued.')
    expect(d).toContain('#6 is the newest, by you.')
    expect(d).toContain('For production you would also type its name')
  })

  it('confirm reads the deployments again, then writes the rollback with an idempotency key', async () => {
    const write = vi.fn(() => ({ status: 201, body: { id: 'dep-7', number: 7 } }))
    const h = await board(
      'manage-rollback',
      {},
      answers(() => devList, write),
    )
    await click(btn('Roll back dev to #5'))
    const clicked = h.calls.length
    const confirm = [...el.querySelectorAll('[role=dialog] button')].find(
      (b) => b.textContent === 'Roll back dev to #5',
    )
    await click(confirm)
    await flush()
    const after = h.calls.slice(clicked)
    const writes = after.filter((c) => c.id === 'iii-cloud::api::write')
    expect(writes).toHaveLength(1)
    expect(writes[0].payload).toMatchObject({ method: 'POST', path: rollbackPath })
    expect(writes[0].payload?.idempotency_key).toMatch(/^[0-9a-f-]{36}$/)
    expect(writes[0].payload).not.toHaveProperty('body')
    // The deployments were read again first.
    const reread = after.findIndex((c) => c.id === 'iii-cloud::api' && c.payload?.path === listPath)
    expect(reread).toBeGreaterThanOrEqual(0)
    expect(reread).toBeLessThan(after.findIndex((c) => c.id === 'iii-cloud::api::write'))
    expect(el.querySelector('[role=dialog]')).toBeNull()
  })

  it('a deploy that started after the dialog opened locks it, and nothing is written', async () => {
    let current: unknown = devList
    const write = vi.fn(() => ({ status: 201, body: {} }))
    const h = await board(
      'manage-rollback',
      {},
      answers(() => current, write),
    )
    await click(btn('Roll back dev to #5'))
    current = newer
    const confirm = () =>
      [...el.querySelectorAll('[role=dialog] button')].find((b) => b.textContent === 'Roll back dev to #5') as
        | HTMLButtonElement
        | undefined
    await click(confirm())
    await flush()
    expect(write).not.toHaveBeenCalled()
    expect(h.calls.some((c) => c.id === 'iii-cloud::api::write')).toBe(false)
    expect(el.querySelector('[role=dialog]')?.textContent).toContain('Deploy #7 started on dev at')
    expect(confirm()?.disabled).toBe(true)
    expect(btn('Open #7')).toBeTruthy()
  })

  it('production: the name has to be typed, and the cloud refusal is shown as it says', async () => {
    const s = boards['manage-rollback'].status
    const prod: Status = {
      ...s,
      project: { ...s.project!, environment: { ...s.project!.environment!, production: true } },
      connection: { ...s.connection, teams: [{ ...s.connection.teams[0], role: 'Developer' }] },
    }
    const write = vi.fn(() => ({
      status: 403,
      body: { code: 'FORBIDDEN', message: 'Only an Owner or Admin can roll back production.' },
    }))
    const h = fakeHost(
      prod,
      answers(() => devList, write),
    )
    await act(async () =>
      root.render(
        <CloudPage
          host={h.host}
          panelSide="right"
          tabId="t"
          {...ctx({ at: 'env', env: DEV.id, tab: 'deploys', deploy: 'dep-5' })}
        />,
      ),
    )
    await flush()
    await click(btn('Roll back dev to #5'))
    expect(el.querySelector('[role=dialog]')?.textContent).toContain(
      'Owners and Admins only; you are Developer on this team.',
    )
    const confirm = () =>
      [...el.querySelectorAll('[role=dialog] button')].find((b) => b.textContent === 'Roll back dev to #5') as
        | HTMLButtonElement
        | undefined
    expect(confirm()?.disabled).toBe(true)
    await type(el.querySelector('input[aria-label="Type dev to confirm"]'), 'de')
    expect(confirm()?.disabled).toBe(true)
    await type(el.querySelector('input[aria-label="Type dev to confirm"]'), 'dev')
    expect(confirm()?.disabled).toBe(false)
    await click(confirm())
    await flush()
    expect(write).toHaveBeenCalledTimes(1)
    expect(el.querySelector('[role=dialog]')?.textContent).toContain('Only an Owner or Admin can roll back production.')
  })

  it('only an earlier live deploy offers a rollback; a stale page locks it', async () => {
    await board('manage-deploys-failed')
    expect(btn('Roll back dev to #2')).toBeFalsy()
    expect(btn('Roll back dev to #6')).toBeFalsy()
    let fail = false
    const s = boards['manage-rollback'].status
    const h = fakeHost(
      () => {
        if (fail) throw new Error('worker gone')
        return s
      },
      cloudAnswers(boards['manage-rollback'].cloud ?? {}),
    )
    await act(async () =>
      root.render(
        <CloudPage
          host={h.host}
          panelSide="right"
          tabId="t"
          {...ctx({ at: 'env', env: DEV.id, tab: 'deploys', deploy: 'dep-5' }, 2)}
        />,
      ),
    )
    await flush()
    expect((btn('Roll back dev to #5') as HTMLButtonElement).disabled).toBe(false)
    fail = true
    await click(btn('Refresh'))
    await flush()
    expect((btn('Roll back dev to #5') as HTMLButtonElement).disabled).toBe(true)
  })
})

/** production: deploy #42 failed after the check; #40 still serves. */
const watch: Status = { ...base, check: { ...ready, checked_at: T(30) }, deploys: [failedDeploy, earlierReady] }

// Moved from the old suite: every behaviour they proved, on the new design.
describe('boards 01-12', () => {
  it('10 watch: the title names the stage, worker and code; the fix composes; the log has the last lines', async () => {
    const h = await mount(watch, ctx({ tab: 'deploys', deployId: failedDeploy.id }))
    expect(text()).toContain('The deploy failed at Workers: registry ran out of memory')
    expect(text()).toContain('The previous release keeps serving.')
    expect(byText('a', 'Open full logs')?.getAttribute('href')).toContain('/deployments/42')
    expect(logText()).toContain('worker registry exited (137)')
    expect(text()).toContain('not reached')
    expect(el.querySelector('[role=tab][aria-selected=true]')?.textContent).toBe('Deploys')
    expect(text()).not.toContain('evidence sent')
    await click(btn('Fix with your agent'))
    const arg = h.compose.mock.calls[0][0]
    expect(arg.text).toContain('Workers')
    expect(arg.text).toContain('failed at Workers')
    expect(arg.submit).toBe(true)
  })
})

describe('replaced deploys, dirty runs and new codes', () => {
  it('a replaced deploy links to its replacement', async () => {
    const s: Status = { ...base, check: ready, pr: { ...pr, state: 'MERGED' }, deploys: [following] }
    const newer = { ...s.deploys[0], id: 'dep-new', commit: { sha: '9d8c7b6a5f4e', message: null }, replaced_by: null }
    const old = {
      ...s.deploys[0],
      outcome: 'replaced' as const,
      status: 'replaced' as const,
      replaced_by: 'dep-new',
      created_at: new Date(Date.parse(s.deploys[0].created_at) - 600_000).toISOString(),
    }
    await mount({ ...s, deploys: [newer, old] }, ctx({ tab: 'deploys', deployId: old.id }))
    expect(text()).toContain('Deploy #41 replaced it before it went live')
    await click(btn('Open that deploy'))
    expect(el.querySelector('[aria-label=Deploys] [aria-current=true]')?.textContent).toContain('9d8c7b6')
    expect(text()).toContain('Deploy #41 · 9d8c7b6')
  })
})

describe('the agent, alive', () => {
  it('the deploy fix hands over with the deploy id', async () => {
    const h = fakeHost(watch, { 'iii-cloud::handoff': { id: 'ho-2', marker: '[m]' } })
    await act(async () =>
      root.render(
        <CloudPage
          host={h.host}
          panelSide="right"
          tabId="t"
          panelContext={{ id: 1, pageId: 'iii-cloud', context: { tab: 'deploys', deployId: 'dep-5f81a09' } }}
        />,
      ),
    )
    await flush()
    await click(btn('Fix with your agent'))
    expect(lastCall(h.calls)).toMatchObject({
      id: 'iii-cloud::handoff',
      payload: { kind: 'deploy_fix', deployment_id: 'dep-5f81a09' },
    })
  })
})

describe('failed deploys, console deploys, projects and direct environments', () => {
  const dep = { tab: 'deploys', deployId: 'dep-5f81a09' }
  const failing = (extra: object = {}) => {
    const s = watch
    const d = {
      ...s.deploys[0],
      failure: {
        code: 'OOMKilled',
        stage: 'workers',
        worker: 'registry',
        title: 'registry ran out of memory in production',
        hint: 'Give it more memory or load the catalog lazily.',
        log_tail: ['[registry] loading the catalog', '[registry] killed: out of memory'],
        ...extra,
      },
    }
    return { ...s, deploys: [d, ...s.deploys.slice(1)] }
  }
  it('a failed deploy: the plain title, the hint, the actions, then the log of the failing stage', async () => {
    const h = await mount(failing(), ctx(dep))
    const card = el.querySelector('.iii-cloud-failure')?.textContent ?? ''
    expect(at('registry ran out of memory in production', card)).toBeGreaterThanOrEqual(0)
    expect(at('registry ran out of memory in production', card)).toBeLessThan(at('Give it more memory', card))
    expect(at('Give it more memory', card)).toBeLessThan(at('Fix with your agent', card))
    expect(at('Fix with your agent')).toBeLessThan(at('killed: out of memory'))
    expect(logText()).toContain('killed: out of memory')
    expect(card).not.toContain('The command exited without printing an error.')
    expect(byText('a', 'Open full logs')?.getAttribute('href')).toContain('/deployments/42')
    await click(btn('Fix with your agent'))
    expect(
      h.calls.some((c) => c.id === 'iii-cloud::handoff' && (c.payload as { kind: string }).kind === 'deploy_fix'),
    ).toBe(true)
  })

  it('a silent failure says so above the hint and the log', async () => {
    await mount(failing({ silent: true, log_tail: [] }), ctx(dep))
    const card = el.querySelector('.iii-cloud-failure')?.textContent ?? ''
    expect(at('The command exited without printing an error.', card)).toBeGreaterThanOrEqual(0)
    expect(at('The command exited without printing an error.', card)).toBeLessThan(at('Give it more memory', card))
    expect(at('The command exited without printing an error.')).toBeLessThan(at('Log'))
  })

  it('a deploy started in the console is labeled as such in the list', async () => {
    const s = watch
    const d = { ...s.deploys[1], from_console: true, pr: null, approved_by_worker: false }
    await mount({ ...s, deploys: [d] }, ctx({ tab: 'deploys' }))
    expect(el.querySelector('[aria-label=Deploys]')?.textContent).toContain('from the console')
  })
})

describe('final core fields', () => {
  it('a deploy that no longer exists says so; a release failure shows its command', async () => {
    const s = watch
    const gone = { ...s.deploys[1], status: 'gone' as const, outcome: 'canceled' as const }
    await mount({ ...s, deploys: [gone] }, ctx({ tab: 'deploys' }))
    expect(text()).toContain('Environment or deployment no longer exists')
    const f = {
      ...s.deploys[0],
      failure: {
        code: 'RELEASE_FAILED',
        stage: 'workers',
        worker: null,
        title: 'The release command failed',
        hint: null,
        log_tail: ['exit 1'],
        silent: true,
        release_command: 'pnpm migrate',
      },
    }
    await mount({ ...s, deploys: [f] }, ctx({ tab: 'deploys', deployId: f.id }, 2))
    expect(text()).toContain('Release command: pnpm migrate')
  })
})

describe('review fixes: usability and copy', () => {
  it('statuses read from one sentence-case map, never lower-cased on the fly', async () => {
    const s = watch
    await mount({ ...s, pr: { ...pr, state: 'MERGED' as const } }, ctx({ tab: 'deploys' }))
    expect(text()).toContain('Failed')
    expect(text()).not.toMatch(/\bno_route\b|\bopen\b chip/)
  })
})
