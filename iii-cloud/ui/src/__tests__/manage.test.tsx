// Manage (U5): the sidebar, Home (Mng1), the phone home (Mng5), the env header and the page states (Mng9).
import { describe, expect, it, vi } from 'vitest'
import { clock } from '../asked'
import { boards } from '../fixtures'
import { agent, apiPath, base, ENV, problems, T, TEAM } from '../fixtures/base'
import { DEV, devDeploys, home, homeCloud, LIVE_SHA, NEW_SHA } from '../fixtures/manage'
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

/** Lets the cloud reads land: the status, each environment's deployments, then the cards' own reads. */
const settle = async () => {
  for (let i = 0; i < 5; i++) await flush()
}
const panel = (title: string) => el.querySelector(`section[aria-label="${title}"]`)
const reads = (h: Awaited<ReturnType<typeof mount>>) =>
  h.calls.filter((c) => c.id === 'iii-cloud::api').map((c) => String(c.payload?.path))
const path = () => el.querySelector('[role=toolbar] nav')?.textContent
/** The sidebar's first row: the project, current at its home. */
const projectRow = () => row('registryiii-hq/registry')
const showHome = async (status: Status, cloud = homeCloud, answers = {}) => {
  const h = await mount(status, ctx({ at: 'home' }), { ...cloudAnswers(cloud), ...answers })
  await settle()
  return h
}

describe('the sidebar', () => {
  it('lists each environment with its live state and the production mark; home is the neutral current page', async () => {
    await board('manage-home')
    await settle()
    const side = el.querySelector('aside[aria-label="registry"]')
    expect(side).not.toBeNull()
    expect(row('production')?.textContent).toBe('productionNothing live yet')
    expect(row('production')?.querySelector('[aria-label=Production]')).not.toBeNull()
    expect(row('dev')?.textContent).toBe(`devLive · ${LIVE_SHA.slice(0, 7)}`)
    expect(row('dev')?.querySelector('[data-dot=ok]')).not.toBeNull()
    expect(row('dev')?.querySelector('[aria-label=Production]')).toBeNull()
    const top = projectRow()
    expect(top?.textContent).toBe('registryiii-hq/registry · team iii')
    expect(top?.getAttribute('aria-current')).toBe('page')
    expect(top?.hasAttribute('data-selected')).toBe(true)
    expect(top?.querySelector('[data-tone=accent], [data-dot=accent]')).toBeNull()
    expect(row('All environments')).toBeUndefined()
    expect(side?.textContent).toContain('Folder ~/workspaces/registry')
    expect(side?.textContent).toContain('Signed in · Developer on team iii')
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
      'Leave registry · dev while it waits for you?Your agent has a question. It keeps waiting; switch back to answer.',
    )
    await click(btn('Stay'))
    expect(h.calls.some((c) => c.id === 'iii-cloud::project::use')).toBe(false)
    expect(projectRow()?.getAttribute('aria-current')).toBe('page')
    await click(row('production'))
    await click(btn('Switch to production'))
    expect(lastCall(h.calls)).toMatchObject({ id: 'iii-cloud::project::use', payload: { environment_id: ENV.id } })
  })

  it('an agent waiting on a wake it armed does not wait on you: no guard, the line says what for', async () => {
    const waiting = { ...home, agent: { ...agent('waiting'), waiting_for: 'the run here to end' } }
    const h = await showHome(waiting)
    expect(el.textContent).toContain('waiting for the run here to end')
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

  it('no project switcher and no add row: Projects in the path is the way to switch and add', async () => {
    const cp = { id: 'p2', name: 'registry-eu', slug: 'registry-eu' }
    const status = { ...base, project: { ...base.project!, cloud_projects: [base.project!.cloud_project!, cp] } }
    await mount(status, ctx({ at: 'home' }))
    expect(el.querySelector('[aria-label^="Switch project"]')).toBeNull()
    expect(el.querySelector('[role=menuitem]')).toBeNull()
    expect(row('Add a project')).toBeUndefined()
    expect(path()).toBe('Projects / registry')
    await click(btn('Projects'))
    expect(path()).toBe('Projects')
    expect(row('registry-eu')).toBeTruthy()
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

  it("a card for a recent project's environment says where it is; Switch to billing takes it up, then home", async () => {
    const h = await mount({ ...base, recent_projects: [billing] }, ctx({ at: 'env', env: 'e-billing-dev' }))
    expect(text()).toContain('dev is in billing')
    expect(text()).not.toContain('Not an environment of this project')
    await click(btn('Switch to billing'))
    expect(h.calls.filter((c) => c.id === 'iii-cloud::project::use').map((c) => c.payload)).toEqual([
      {
        path: '/home/you/workspaces/billing',
        cloud_project_id: 'p-billing',
        environment_id: 'e-billing-dev',
        mode: 'checkout',
      },
    ])
    expect(projectRow()?.getAttribute('aria-current')).toBe('page')
  })

  it('iii cloud wants a fresh sign-in: no Switch, only where the environment is', async () => {
    const problem = { ...problems[0], code: 'SIGN_IN_REQUIRED' }
    const status = { ...base, check: { ...base.check!, problems: [problem] }, recent_projects: [billing] }
    const h = await mount(status, ctx({ at: 'env', env: 'e-billing-dev' }))
    expect(text()).toContain('dev is in billing')
    expect(btn('Switch to billing')).toBeNull()
    expect(h.calls.some((c) => c.id === 'iii-cloud::project::use')).toBe(false)
  })

  it('another cloud project of this folder holds it: Switch to it keeps the folder', async () => {
    const eu = {
      id: 'p2',
      name: 'registry-eu',
      slug: 'registry-eu',
      environments: [{ id: 'e-eu', name: 'eu', branch: 'main' }],
    }
    const status = { ...base, project: { ...base.project!, cloud_projects: [base.project!.cloud_project!, eu] } }
    const h = await mount(status, ctx({ at: 'env', env: 'e-eu' }))
    expect(text()).toContain('eu is in registry-eu')
    await click(btn('Switch to registry-eu'))
    expect(h.calls.find((c) => c.id === 'iii-cloud::project::use')?.payload).toEqual({
      path: '/home/you/workspaces/registry',
      cloud_project_id: 'p2',
      environment_id: 'e-eu',
      mode: 'worktree',
    })
  })
})

describe('Home (Mng1)', () => {
  it('one card per environment from iii cloud: serves, deploys, app file, health, agent, who', async () => {
    const h = await board('manage-home')
    await settle()
    expect(el.querySelector('[role=toolbar] h2')?.textContent).toBe('registry')
    expect(el.querySelector('[role=toolbar]')?.textContent).toContain('2 environments')
    const dev = panel('dev')?.textContent ?? ''
    expect(dev).toContain(`Serves${LIVE_SHA.slice(0, 7)} · deploy #6 · ${clock(T(5))} by you`)
    expect(dev).toContain('Deploysfrom develop: here after the check, or on any push')
    expect(dev).toContain('App fileapi/worker-compose-prod.yaml · 11 workers')
    expect(dev).toContain(`Health11 of 11 workers up · checked ${clock(T(1))}`)
    expect(dev).toContain(`Your agentidle · last check passed ${clock(T(1))}`)
    expect(dev).not.toContain('Who')
    expect(panel('dev')?.querySelector('[data-chip]')?.textContent).toBe('Live')
    const prod = panel('production')?.textContent ?? ''
    expect(prod).toContain('Servesnothing yet')
    expect(prod).toContain('Deployswhen a pull request into main is merged')
    expect(prod).toContain("WhoOwners and Admins merge it; you're a Developer")
    expect(prod).not.toContain('App file')
    expect(prod).not.toContain('Your agent')
    expect(panel('production')?.textContent).toContain('Production')
    // The cards of what serves nothing read no machines and no health.
    expect(
      reads(h)
        .filter((p) => p.endsWith('/machines') || p.endsWith('/health'))
        .sort(),
    ).toEqual([apiPath('/health', DEV.id), apiPath('/machines', DEV.id)].sort())
    // No Open button: the card's title opens the environment.
    expect(byText('button', 'Open dev')).toBeUndefined()
    await click(panel('dev')?.querySelector('button'))
    expect(path()).toBe('Projects / registry / dev')
  })

  it('recent deploys of every environment: who started each, superseded, and "checked here" only for ours', async () => {
    await board('manage-home')
    await settle()
    const rows = [...(panel('Recent deploys, all environments')?.querySelectorAll('tbody tr') ?? [])].map((r) =>
      [...r.querySelectorAll('td')].map((td) => td.textContent),
    )
    expect(rows).toEqual([
      ['#6', 'dev', `${LIVE_SHA.slice(0, 7)} Search: rank by stars`, 'Live', `${clock(T(6))} · you · checked here`],
      ['#5', 'dev', 'd738d2f Drop the separate web container', 'Superseded', `${clock(T(60))} · ana`],
      ['#2', 'dev', '86c49e1 Ready the registry', 'Failed', `${clock(T(90))} · a push`],
    ])
    await click(btn('All deploys of dev'))
    expect(el.querySelector('[role=tab][aria-selected=true]')?.textContent).toBe('Deploys')
  })

  it('the waiting panel: the check passed on a commit dev does not serve; Review and deploy opens the review', async () => {
    await board('manage-home')
    await settle()
    const wait = byText('[role=status] b', `Waiting for you: dev can get ${NEW_SHA.slice(0, 7)}`)?.parentElement
    expect(wait?.getAttribute('data-variant')).toBe('warn')
    expect(wait?.textContent).toContain(
      `It has 1 commit dev doesn't serve, ${NEW_SHA.slice(0, 7)} "Search: tweak weights". The check passed on iii cloud and here at ${clock(T(1))}.`,
    )
    await click(btn('Review and deploy'))
    expect(path()).toBe('Projects / registry / dev / Deploy')
  })

  it('no waiting panel when the check is not ready', async () => {
    await showHome({ ...home, check: { ...home.check!, ready: false } })
    expect(text()).not.toContain('Waiting for you')
  })

  it('no waiting panel when dev already serves the checked commit', async () => {
    await showHome({ ...home, check: { ...home.check!, commit: LIVE_SHA } })
    expect(text()).not.toContain('Waiting for you')
  })

  it('no waiting panel while a deploy runs; the environment says Deploying', async () => {
    const running = { ...devDeploys[0], id: 'dep-7', number: 7, status: 'in_progress', current: false }
    await showHome(home, { ...homeCloud, [apiPath('/deployments', DEV.id)]: { deployments: [running, ...devDeploys] } })
    expect(text()).not.toContain('Waiting for you')
    expect(row('dev')?.textContent).toBe(`devDeploying · ${LIVE_SHA.slice(0, 7)}`)
  })

  it('the env header: the env state, its address, as of and Refresh', async () => {
    const h = await mount(home, ctx({ at: 'env', env: DEV.id }), cloudAnswers(homeCloud))
    await settle()
    const bar = el.querySelector('[role=toolbar]')
    expect(bar?.querySelector('nav')?.textContent).toBe('Projects / registry / dev')
    expect(bar?.querySelector('[data-chip]')?.textContent).toBe('Live')
    expect(bar?.textContent).toContain('registry-dev.iii.cloud')
    expect(bar?.textContent).toContain(`as of ${clock(home.refreshed_at!)}`)
    await click(btn('Refresh'))
    expect(h.calls.filter((c) => c.id === 'iii-cloud::status').at(-1)?.payload).toEqual({ refresh: true })
  })
})

describe('the phone home (Mng5)', () => {
  it('three segments: environments with their state, the deploys of all of them, the checks', async () => {
    ui.narrow = true
    await board('manage-home')
    await settle()
    expect(el.querySelector('aside')).toBeNull()
    const segs = [...el.querySelectorAll('[data-segmented] [role=tab]')].map((t) => t.textContent)
    expect(segs).toEqual(['Environments', 'Deploys', 'Checks'])
    expect(text()).toContain(`Waiting for you: dev can get ${NEW_SHA.slice(0, 7)}`)
    expect(row('production')?.textContent).toContain('Nothing live yet')
    expect(row('dev')?.textContent).toContain(`${LIVE_SHA.slice(0, 7)} · deploy #6`)
    expect(panel('Recent on dev')).not.toBeNull()
    await click(byText('[data-segmented] [role=tab]', 'Deploys'))
    expect(
      [...(panel('Recent deploys')?.querySelectorAll('[role=row]') ?? [])].map((r) => r.firstChild?.textContent),
    ).toEqual(['#6', '#5', '#2'])
    await click(row('#2'))
    expect(el.querySelector('[role=toolbar] h2')?.textContent).toBe('dev')
    expect(el.querySelector('[data-segmented] [aria-selected=true]')?.textContent).toBe('Deploys')
  })

  it('the header: the project with a way back to Projects, no project menu', async () => {
    ui.narrow = true
    await board('manage-home')
    await settle()
    expect(el.querySelector('[role=toolbar] h2')?.textContent).toBe('registry')
    expect(el.querySelector('[role=menuitem]')).toBeNull()
    await click(btn('Back to Projects'))
    expect(el.querySelector('[role=toolbar] h2')?.textContent).toBe('Projects')
  })

  it('an environment opens over the home with its four tabs as segments; Back returns', async () => {
    ui.narrow = true
    await board('manage-home')
    await settle()
    await click(row('dev'))
    const segs = [...el.querySelectorAll('[data-segmented] [role=tab]')].map((t) => t.textContent)
    expect(segs).toEqual(['Overview', 'Deploys', 'Settings', 'Checks'])
    expect(el.querySelector('[role=tablist]:not([data-segmented])')).toBeNull()
    await click(btn('Back to registry'))
    expect([...el.querySelectorAll('[data-segmented] [role=tab]')].map((t) => t.textContent)).toEqual([
      'Environments',
      'Deploys',
      'Checks',
    ])
  })
})

describe('page states (Mng9)', () => {
  it('empty: no environments yet, and the way to set one up', async () => {
    await board('manage-empty')
    expect(text()).toContain('registry has no environments yet')
    expect(text()).toContain('Set one up: the worker reads this folder and checks it before the first deploy.')
    await click(btn('Set up an environment'))
    expect(el.querySelector('aside[aria-label="Setup steps"]')).not.toBeNull()
  })

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
    expect((btn('Deploy from this machine') as HTMLButtonElement).disabled).toBe(false)
    down = true
    await click(btn('Refresh'))
    const note = byText('[role=status] b', `iii cloud isn't answering · showing what it said at ${clock(T(9))}`)
    expect(note?.parentElement?.textContent).toContain('Deploy, rollback and saves stay locked.')
    expect(el.querySelector('[data-stale]')).not.toBeNull()
    expect((btn('Deploy from this machine') as HTMLButtonElement).disabled).toBe(true)
    expect((btn('Review and deploy') as HTMLButtonElement).disabled).toBe(true)
    expect(text()).toContain('Search: rank by stars')
    down = false
    await click(btn('Try again'))
    expect(text()).not.toContain("iii cloud isn't answering")
    expect((btn('Deploy from this machine') as HTMLButtonElement).disabled).toBe(false)
  })

  it('deleted: the project is gone in iii cloud, with the restore date and the way to the console', async () => {
    const h = await board('manage-deleted')
    await settle()
    const note = byText('[role=status] b', 'registry was deleted in iii cloud')?.parentElement
    expect(note?.getAttribute('data-variant')).toBe('alert')
    const until = new Date(Date.UTC(2026, 11, 3, 12)).toLocaleDateString([], { day: 'numeric', month: 'short' })
    expect(note?.textContent).toContain(
      `It can be restored until ${until} in the iii cloud console. Nothing here can deploy to it.`,
    )
    expect(byText('a', 'Open in iii cloud')?.getAttribute('href')).toBe('https://cloud.iii.dev/iii')
    expect(reads(h)).toEqual([`/teams/${TEAM.id}/deletions`])
    expect(text()).not.toContain('Recent deploys')
  })

  it('deleted environment: its page says so in place of the tab; the sidebar stays', async () => {
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
