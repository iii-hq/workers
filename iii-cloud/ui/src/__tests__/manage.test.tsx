// Manage (U5): the project's sidebar around an environment, an environment of another project, the page states (Mng9).
// The home, the phone home and the env cards are gone: the project opens on its environment.
import { describe, expect, it, vi } from 'vitest'
import { clock } from '../asked'
import { boards } from '../fixtures'
import { agent, apiPath, base, ENV, problems, T, TEAM } from '../fixtures/base'
import { DEV, home, homeCloud, LIVE_SHA, NEW_SHA } from '../fixtures/manage'
import { shortDate } from '../model'
import type { Status } from '../types'
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
  row,
  text,
  ui,
  usePage,
} from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

/** Lets the cloud reads land: the status, each environment's deployments, then the page's own reads. */
const settle = async () => {
  for (let i = 0; i < 5; i++) await flush()
}
const panel = (title: string) => el.querySelector(`section[aria-label="${title}"]`)
const reads = (h: Awaited<ReturnType<typeof mount>>) =>
  h.calls.filter((c) => c.id === 'iii-cloud::api').map((c) => String(c.payload?.path))
const path = () => el.querySelector('[role=toolbar] nav')?.textContent
const showHome = async (status: Status, cloud = homeCloud, answers = {}) => {
  const h = await mount(status, ctx({ at: 'home' }), { ...cloudAnswers(cloud), ...answers })
  await settle()
  return h
}

describe('the sidebar', () => {
  it('lists each environment with its state line and the production mark; the selected one is the page', async () => {
    await board('manage-home')
    await settle()
    const side = el.querySelector('aside[aria-label="registry"]')
    expect(side).not.toBeNull()
    expect(row('production')?.textContent).toBe('productionNot deployed yet')
    expect(row('production')?.querySelector('[aria-label=Production]')).not.toBeNull()
    expect(row('dev')?.textContent).toBe(`devLive · ${LIVE_SHA.slice(0, 7)}`)
    expect(row('dev')?.querySelector('[data-dot=ok]')).not.toBeNull()
    expect(row('dev')?.querySelector('[aria-label=Production]')).toBeNull()
    expect(row('dev')?.getAttribute('aria-current')).toBe('page')
    expect(row('dev')?.hasAttribute('data-selected')).toBe(true)
    expect(side?.textContent).toContain('iii-hq/registry · team iii')
    expect(side?.textContent).toContain('Folder ~/workspaces/registry')
    expect(side?.textContent).toContain('you@example.com · Developer')
  })

  it('an environment that is not selected is selected with project::use {environment_id}', async () => {
    const h = await board('manage-home')
    await click(row('production'))
    expect(lastCall(h.calls)).toMatchObject({
      id: 'iii-cloud::project::use',
      payload: { environment_id: ENV.id, path: '/home/you/workspaces/registry', mode: 'worktree' },
    })
    expect(row('production')?.getAttribute('aria-current')).toBe('page')
    expect(row('production')?.hasAttribute('data-selected')).toBe(true)
  })

  it('another environment while something waits on you: the guard asks first; Stay keeps this one', async () => {
    const waiting = { ...home, agent: { ...agent('waiting_for_you') } }
    const h = await showHome(waiting)
    await click(row('production'))
    expect(el.querySelector('[role=alertdialog]')?.textContent).toContain(
      'Leave registry · dev while it waits for you?Your agent has a question. It keeps waiting; open it again to answer.',
    )
    await click(btn('Stay'))
    expect(h.calls.some((c) => c.id === 'iii-cloud::project::use')).toBe(false)
    expect(row('dev')?.getAttribute('aria-current')).toBe('page')
    await click(row('production'))
    await click(btn('Open production'))
    expect(lastCall(h.calls)).toMatchObject({ id: 'iii-cloud::project::use', payload: { environment_id: ENV.id } })
  })

  it('an agent waiting on a wake it armed does not wait on you: no guard', async () => {
    const waiting = { ...home, agent: { ...agent('waiting'), waiting_for: 'the local run to end' } }
    const h = await showHome(waiting)
    await click(row('production'))
    expect(el.querySelector('[role=alertdialog]')).toBeNull()
    expect(lastCall(h.calls)).toMatchObject({ id: 'iii-cloud::project::use', payload: { environment_id: ENV.id } })
  })

  it('reads /deployments once per environment and refresh, never more', async () => {
    const h = await board('manage-home')
    await settle()
    const deployments = () => reads(h).filter((p) => p.endsWith('/deployments'))
    expect(deployments().sort()).toEqual([apiPath('/deployments'), apiPath('/deployments', DEV.id)].sort())
    await click(row('dev'))
    await settle()
    expect(deployments()).toHaveLength(2)
  })

  it('no project switcher and no add row: Projects in the path is the way to open and add', async () => {
    const cp = { id: 'p2', name: 'registry-eu', slug: 'registry-eu' }
    const status = { ...base, project: { ...base.project!, cloud_projects: [base.project!.cloud_project!, cp] } }
    await mount(status, ctx({ at: 'home' }))
    expect(el.querySelector('[aria-label^="Switch project"]')).toBeNull()
    expect(el.querySelector('[role=menuitem]')).toBeNull()
    expect(row('Add a project')).toBeUndefined()
    expect(path()).toBe('Projects / registry · production')
    await click(btn('Projects'))
    expect(path()).toBe('Projects')
    expect(row('registry-eu')).toBeTruthy()
  })

  it('a narrow pane: the environments are tabs, each with its dot; a tab opens that environment', async () => {
    ui.narrow = true
    await board('manage-home')
    await settle()
    expect(el.querySelector('aside')).toBeNull()
    expect([...el.querySelectorAll('[data-segmented] [role=tab]')].map((t) => t.textContent)).toEqual([
      'production',
      'dev',
    ])
    expect(el.querySelector('[data-segmented] [aria-selected=true]')?.textContent).toBe('dev')
    expect(el.querySelector('[role=toolbar] h2')?.textContent).toBe('registry · dev')
    await click(btn('Back to Projects'))
    expect(el.querySelector('[role=toolbar] h2')?.textContent).toBe('Projects')
  })
})

describe('an environment of another project', () => {
  const billing = {
    path: '/home/you/workspaces/billing',
    repository: 'iii-hq/billing',
    cloud_project_id: 'p-billing',
    cloud_project_name: 'billing',
    environment_id: 'e-billing-dev',
    environment_name: 'dev',
    mode: 'checkout' as const,
    used_at: T(60),
  }

  it("a card for a recent project's environment says where it is; Open billing takes it up, then its environment", async () => {
    const h = await mount({ ...base, recent_projects: [billing] }, ctx({ at: 'env', env: 'e-billing-dev' }))
    expect(text()).toContain('dev is in billing')
    expect(text()).not.toContain('Not an environment of this project')
    await click(btn('Open billing'))
    expect(h.calls.filter((c) => c.id === 'iii-cloud::project::use').map((c) => c.payload)).toEqual([
      {
        path: '/home/you/workspaces/billing',
        cloud_project_id: 'p-billing',
        environment_id: 'e-billing-dev',
        mode: 'checkout',
      },
    ])
    expect(row('production')?.getAttribute('aria-current')).toBe('page')
  })

  it('iii cloud wants a fresh sign-in: no Open, only where the environment is', async () => {
    const problem = { ...problems[0], code: 'SIGN_IN_REQUIRED' }
    const status = { ...base, check: { ...base.check!, problems: [problem] }, recent_projects: [billing] }
    const h = await mount(status, ctx({ at: 'env', env: 'e-billing-dev' }))
    expect(text()).toContain('dev is in billing')
    expect(btn('Open billing')).toBeNull()
    expect(h.calls.some((c) => c.id === 'iii-cloud::project::use')).toBe(false)
  })

  it('another cloud project of this folder holds it: Open it keeps the folder', async () => {
    const eu = {
      id: 'p2',
      name: 'registry-eu',
      slug: 'registry-eu',
      environments: [{ id: 'e-eu', name: 'eu', branch: 'main' }],
    }
    const status = { ...base, project: { ...base.project!, cloud_projects: [base.project!.cloud_project!, eu] } }
    const h = await mount(status, ctx({ at: 'env', env: 'e-eu' }))
    expect(text()).toContain('eu is in registry-eu')
    await click(btn('Open registry-eu'))
    expect(h.calls.find((c) => c.id === 'iii-cloud::project::use')?.payload).toEqual({
      path: '/home/you/workspaces/registry',
      cloud_project_id: 'p2',
      environment_id: 'e-eu',
      mode: 'worktree',
    })
  })
})

describe('the environment page', () => {
  it('the waiting panel: the check passed on a commit dev does not serve; Review and deploy opens the check and deploy screen', async () => {
    await board('manage-home')
    await settle()
    const wait = byText('[role=status] b', `Waiting for you: 1 commit passed the check`)?.parentElement
    expect(wait?.getAttribute('data-variant')).toBe('warn')
    expect(wait?.textContent).toContain(
      `${NEW_SHA.slice(0, 7)} "Search: tweak weights" · checked on iii cloud and here`,
    )
    await click(btn('Review and deploy'))
    expect(path()).toBe('Projects / registry · dev / Deploy')
  })

  it('no waiting panel when the check is not ready, or dev already serves the checked commit', async () => {
    await showHome({ ...home, check: { ...home.check!, ready: false } })
    expect(text()).not.toContain('Waiting for you')
    await showHome({ ...home, check: { ...home.check!, commit: LIVE_SHA } })
    expect(text()).not.toContain('Waiting for you')
  })

  it('the header: the state, its address, as of and Refresh', async () => {
    const h = await mount(home, ctx({ at: 'env', env: DEV.id }), cloudAnswers(homeCloud))
    await settle()
    const bar = el.querySelector('[role=toolbar]')
    expect(bar?.querySelector('nav')?.textContent).toBe('Projects / registry · dev')
    expect(bar?.querySelector('[data-chip]')?.textContent).toBe('Live')
    expect(bar?.textContent).toContain('registry-dev.iii.cloud')
    expect(bar?.textContent).toContain(`as of ${clock(home.refreshed_at!)}`)
    await click(btn('Refresh'))
    expect(h.calls.filter((c) => c.id === 'iii-cloud::status').at(-1)?.payload).toEqual({ refresh: true })
  })
})

describe('page states (Mng9)', () => {
  it('out of date: the worker stops answering; the page keeps what it had, locks deploys, and tries again', async () => {
    const b = boards['manage-stale']
    let down = false
    const status = () => {
      if (down) throw new Error('worker gone')
      return b.status
    }
    // `mount` hands the status to fakeHost, which reads a function on every call.
    await mount(status as unknown as Status, ctx({ at: 'env', env: DEV.id }), cloudAnswers(b.cloud ?? {}))
    await settle()
    expect((btn('Deploy to dev') as HTMLButtonElement).disabled).toBe(false)
    down = true
    await click(btn('Refresh'))
    const note = byText('[role=status] b', `iii cloud isn't answering · showing what it said at ${clock(T(9))}`)
    expect(note?.parentElement?.textContent).toContain('Deploying and saves stay locked.')
    expect(el.querySelector('[data-stale]')).not.toBeNull()
    expect((btn('Deploy to dev') as HTMLButtonElement).disabled).toBe(true)
    expect((btn('Review and deploy') as HTMLButtonElement).disabled).toBe(true)
    expect(text()).toContain('Search: rank by stars')
    down = false
    await click(btn('Try again'))
    expect(text()).not.toContain("iii cloud isn't answering")
    expect((btn('Deploy to dev') as HTMLButtonElement).disabled).toBe(false)
  })

  it('deleted: the project is gone in iii cloud, with the restore date and the way to the console', async () => {
    const h = await board('manage-deleted')
    await settle()
    const note = byText('[role=status] b', 'registry was deleted in iii cloud')?.parentElement
    expect(note?.getAttribute('data-variant')).toBe('alert')
    const until = shortDate(new Date(Date.UTC(2026, 11, 3, 12)).toISOString())
    expect(note?.textContent).toContain(
      `It can be restored until ${until} in the iii cloud console. Nothing here can deploy to it.`,
    )
    expect(byText('a', 'Open in iii cloud')?.getAttribute('href')).toBe('https://cloud.iii.dev/iii')
    expect(reads(h)).toEqual([`/teams/${TEAM.id}/deletions`])
  })

  it('deleted environment: its page says so; the sidebar stays', async () => {
    const status = { ...home, problems: [{ code: 'ENVIRONMENT_DELETED', message: 'gone' }] }
    const deletions = {
      deletions: [{ kind: 'environment', environment_id: DEV.id, restorable: false, restorable_until: null }],
    }
    await mount(
      status,
      ctx({ at: 'env', env: DEV.id }),
      cloudAnswers({ ...homeCloud, [`/teams/${TEAM.id}/deletions`]: deletions }),
    )
    await settle()
    const note = byText('[role=status] b', 'dev was deleted in iii cloud')?.parentElement
    expect(note?.textContent).toContain('Nothing here can deploy to it.')
    expect(note?.textContent).not.toContain('restored')
    expect(panel('Serving')).toBeNull()
    expect(row('production')).toBeTruthy()
  })

  it('signed out: iii cloud wants a fresh sign-in; Sign in again starts the browser sign-in', async () => {
    const problem = { ...problems[0], code: 'SIGN_IN_REQUIRED' }
    const status = { ...home, check: { ...home.check!, problems: [problem] } }
    const h = await showHome(status)
    const note = byText('[role=status] b', 'Sign in to iii cloud again')?.parentElement
    expect(note?.getAttribute('data-variant')).toBe('alert')
    expect((btn('Review and deploy') as HTMLButtonElement).disabled).toBe(true)
    await click(btn('Sign in again'))
    expect(h.calls.filter((c) => c.id === 'iii-cloud::connect')).toEqual([
      { id: 'iii-cloud::connect', payload: { method: 'browser' } },
    ])
  })
})
