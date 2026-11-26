// Start (§3 of the navigation plan): the account, the projects on this machine, Switch and its guard.
import { describe, expect, it, vi } from 'vitest'
import { base, ENV, request, signedOut, TEAM } from '../fixtures/base'
import { ACME, billing, startCloud, startStatus } from '../fixtures/start'
import type { RecentProject, Status } from '../types'
import type { Call } from './host'
import { board, btn, click, cloudAnswers, ctx, el, flush, mount, text, ui, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

const settle = async () => {
  for (let i = 0; i < 5; i++) await flush()
}
const rows = () => [...el.querySelectorAll('[aria-label="Projects on this machine"] [role=row]')]
const names = () => rows().map((r) => r.querySelector('.iii-cloud-start-name')?.firstChild?.textContent)
/** A row by its project's name, and its live line and folder line. */
const place = (name: string) =>
  rows().find((r) => r.querySelector('.iii-cloud-start-name')?.firstChild?.textContent === name)
const lines = (name: string) =>
  [...(place(name)?.querySelectorAll('.iii-cloud-start-lines > span') ?? [])].map((s) => s.textContent)
const switchOf = (name: string) =>
  place(name)?.querySelector('button[aria-label^="Switch the worker"]') as HTMLButtonElement | null
const uses = (h: { calls: Call[] }) => h.calls.filter((c) => c.id === 'iii-cloud::project::use').map((c) => c.payload)
const reads = (h: { calls: Call[] }) =>
  h.calls.filter((c) => c.id === 'iii-cloud::api').map((c) => String(c.payload?.path))
const path = () => el.querySelector('[role=toolbar] nav')?.textContent
const showStart = async (status: Status = startStatus, answers = {}) => {
  const h = await mount(status, ctx({ at: 'start' }), { ...cloudAnswers(startCloud), ...answers })
  await settle()
  return h
}

describe('Start (desktop)', () => {
  it('the path, the account, the rows in order (in use, recent, this folder’s other projects), one per folder and project', async () => {
    await board('start')
    await settle()
    expect(path()).toBe('Projects')
    expect(el.querySelector('[role=toolbar]')?.textContent).toContain('on this machine')
    // The account as a label and its value.
    expect(el.querySelector('[aria-label=Account] .iii-cloud-start-key')?.textContent).toBe('Signed in')
    expect(el.querySelector('[aria-label=Account] .iii-cloud-start-val')?.textContent).toBe(
      'ana@iii.dev · teams iii (Developer), acme (Owner)',
    )
    // The header's add actions: New project in iii cloud first, both 28 px.
    const adds = [...el.querySelectorAll('.iii-cloud-panel-head button')]
    expect(adds.map((b) => b.textContent)).toEqual(['New project in iii cloud…', 'Add a project…'])
    expect(adds.every((b) => b.classList.contains('iii-cloud-start-headbtn'))).toBe(true)
    expect(names()).toEqual(['registry', 'billing', 'registry-3', 'old', 'registry-eu'])
    // Line 2: the folder (mono), how the worker works in it, the team; no repository.
    expect(lines('billing')[1]).toBe('~/workspaces/billing · edits this folder · team acme')
    expect(lines('registry')[1]).toBe('~/workspaces/registry · separate copy · team iii')
    expect(place('registry')?.querySelector('.iii-cloud-start-lines .iii-cloud-mono')?.textContent).toBe(
      '~/workspaces/registry',
    )
    // The row in use: "In use" beside its name, what it serves in line 1, and Open alone at its end.
    expect(place('registry')?.querySelector('.iii-cloud-start-name [data-chip]')?.textContent).toBe('In use')
    expect(place('registry')?.querySelectorAll('[data-chip]')).toHaveLength(1)
    expect([...(place('registry')?.querySelectorAll('button') ?? [])].map((b) => b.textContent)).toEqual(['Open'])
    expect(text()).toContain('Switching moves this worker to that folder and environment')
  })

  it('at most nine rows', async () => {
    const many: RecentProject[] = Array.from({ length: 12 }, (_, i) => ({ ...billing, path: `/home/you/w/p${i}` }))
    await showStart({ ...startStatus, recent_projects: many })
    expect(rows()).toHaveLength(9)
  })

  it('live lines from iii cloud: what the env serves, what deploys, what failed, how many environments', async () => {
    const h = await board('start')
    await settle()
    expect(lines('registry')[0]).toMatch(/^production · serves 1a2b3c4 · deploy #40 · .+ · 1 environment$/)
    expect(lines('billing')[0]).toBe('dev · deploying #8 · 2 environments')
    expect(lines('registry-eu')[0]).toMatch(/^eu · deploy #3 failed .+ · 1 environment$/)
    // registry-3's environment is gone: its row reads production.
    expect(lines('registry-3')[0]).toBe('production · nothing live yet · 1 environment')
    expect(reads(h)).toContain(`/teams/${ACME.id}/projects/p-billing/environments`)
    expect(reads(h)).toContain(`/teams/${ACME.id}/environments/e-billing-dev/deployments`)
    // The project in use reads only its environment's deployments: its environments are in the status.
    expect(reads(h)).not.toContain(`/teams/${TEAM.id}/projects/p1/environments`)
  })

  it('Switch takes the project up with project::use, then its home', async () => {
    const h = await board('start')
    await settle()
    expect(switchOf('billing')?.getAttribute('aria-label')).toBe('Switch the worker to billing in ~/workspaces/billing')
    await click(switchOf('billing'))
    expect(uses(h)).toEqual([
      {
        path: '/home/you/workspaces/billing',
        cloud_project_id: 'p-billing',
        environment_id: 'e-billing-dev',
        mode: 'checkout',
      },
    ])
    expect(path()).toBe('Projects / registry · production')
  })

  it('until iii cloud lists the environments, Switch is off and busy', async () => {
    const answers = cloudAnswers(startCloud)
    const api = (q: Record<string, unknown>) =>
      String(q.path).includes('p-billing') ? new Promise(() => undefined) : answers['iii-cloud::api'](q)
    const h = await showStart(startStatus, { 'iii-cloud::api': api })
    expect(switchOf('billing')?.disabled).toBe(true)
    expect(switchOf('billing')?.getAttribute('aria-busy')).toBe('true')
    expect(switchOf('registry-3')?.disabled).toBe(false)
    expect(switchOf('registry-3')?.hasAttribute('aria-busy')).toBe(false)
    await click(switchOf('billing'))
    expect(uses(h)).toEqual([])
  })

  it('an environment iii cloud no longer lists: Switch sends no environment_id', async () => {
    const h = await board('start')
    await settle()
    await click(switchOf('registry-3'))
    expect(uses(h)).toEqual([{ path: '/home/you/workspaces/registry-3', cloud_project_id: 'p-r3', mode: 'worktree' }])
  })

  it('the default environment taken instead: the home says so', async () => {
    const answer = () => ({
      ...base.project,
      environment: { ...ENV, manifest: null, url: null },
      fell_back_from: 'e-billing-dev',
    })
    await showStart(startStatus, { 'iii-cloud::project::use': answer })
    await click(switchOf('billing'))
    expect(text()).toContain('dev no longer exists; showing production')
  })

  it('deleted in iii cloud: the Deleted chip, until when it can be restored, the way to iii cloud, and no Switch', async () => {
    await board('start')
    await settle()
    expect(place('old')?.querySelector('[data-chip]')?.textContent).toBe('Deleted')
    const until = '3 Dec'
    expect(lines('old')[0]).toBe(`restorable until ${until}`)
    expect(place('old')?.querySelector('a')?.textContent).toBe('Open in iii cloud')
    expect(switchOf('old')).toBeNull()
  })

  it('a read that fails says so in the row; Switch stays', async () => {
    const fail = () => ({ status: 500, body: { code: 'CLOUD_ERROR', message: '' } })
    const answers = cloudAnswers(startCloud)
    const api = (q: Record<string, unknown>) =>
      String(q.path).includes('p-billing') ? fail() : answers['iii-cloud::api'](q)
    await showStart(startStatus, { 'iii-cloud::api': api })
    expect(lines('billing')[0]).toBe("Can't read iii cloud: iii cloud had a problem. Try again in a moment.")
    expect(switchOf('billing')?.disabled).toBe(false)
  })

  it('something waits on the person: the guard asks first, and Stay sends nothing', async () => {
    const waiting = { ...startStatus, secrets: [request('SESSION_SECRET', 'ask', 'waiting', '')] }
    const h = await showStart(waiting)
    await click(switchOf('billing'))
    const dialog = el.querySelector('[role=alertdialog]')
    expect(dialog?.textContent).toContain('Leave registry · production while it waits for you?')
    expect(dialog?.textContent).toContain('1 secret is waiting. It keeps waiting; open it again to answer.')
    expect(btn('Open billing')).toBeTruthy()
    await click(btn('Stay'))
    expect(el.querySelector('[role=alertdialog]')).toBeNull()
    expect(uses(h)).toEqual([])
    await click(switchOf('billing'))
    await click(btn('Open billing'))
    expect(uses(h)).toHaveLength(1)
  })

  it('the guard confirmed after the page went out of date sends nothing', async () => {
    let down = false
    const waiting = { ...startStatus, secrets: [request('SESSION_SECRET', 'ask', 'waiting', '')] }
    const status = () => {
      if (down) throw new Error('worker gone')
      return waiting
    }
    const h = await showStart(status as unknown as Status)
    await click(switchOf('billing'))
    expect(el.querySelector('[role=alertdialog]')).not.toBeNull()
    down = true
    await click(btn('Refresh'))
    expect(text()).toContain("iii cloud isn't answering")
    await click(btn('Open billing'))
    expect(el.querySelector('[role=alertdialog]')).toBeNull()
    expect(uses(h)).toEqual([])
  })

  it('a switch that fails: the error above the list, the page stays, the row in use unchanged', async () => {
    const gone = Object.assign(new Error('billing no longer exists in iii cloud.'), { code: 'PROJECT_DELETED' })
    await showStart(startStatus, { 'iii-cloud::project::use': gone })
    await click(switchOf('billing'))
    expect(path()).toBe('Projects')
    expect(el.querySelector('[role=status][data-variant=alert]')?.textContent).toContain(
      'billing no longer exists in iii cloud.',
    )
    expect(place('registry')?.textContent).toContain('In use')
    expect(switchOf('billing')?.disabled).toBe(false)
  })

  it('signed out: no reads, the way to sign in, no live state, and Switch off', async () => {
    const h = await showStart({ ...startStatus, connection: signedOut })
    expect(reads(h)).toEqual([])
    expect(text()).toContain('Not signed in to iii cloud')
    expect(lines('billing')[0]).toBe("Sign in to see what's live")
    expect(switchOf('billing')?.disabled).toBe(true)
    await click(btn('Sign in with your browser'))
    expect(h.calls.filter((c) => c.id === 'iii-cloud::connect').map((c) => c.payload)).toEqual([{ method: 'browser' }])
  })

  it('an API key: one row read through the key’s environment, no add actions, and the browser sign-in offered', async () => {
    const key = { secret: 'IIICLOUD_API_KEY', team_id: TEAM.id, environment_id: ENV.id }
    const status = {
      ...startStatus,
      connection: { ...startStatus.connection, method: 'api_key' as const, api_key: key },
      project: { ...startStatus.project!, team: null, cloud_project: null },
    }
    const h = await showStart(status)
    expect(names()).toEqual(['iii-hq/registry'])
    expect(reads(h)).toEqual([`/teams/${TEAM.id}/environments/${ENV.id}/deployments`])
    expect(el.querySelector('[aria-label=Account] .iii-cloud-start-key')?.textContent).toBe('API key')
    expect(text()).toContain('IIICLOUD_API_KEY · reaches one environment')
    expect(btn('Add a project…')).toBeNull()
    expect(btn('New project in iii cloud…')).toBeNull()
    expect(btn('Sign in with your browser')).toBeTruthy()
  })

  it('the add actions open setup in add mode; Open goes home with no call', async () => {
    const h = await board('start')
    await settle()
    await click(btn('New project in iii cloud…'))
    expect(path()).toBe('Projects / Add a project')
    await click(btn('Projects'))
    await click(btn('Open'))
    expect(path()).toBe('Projects / registry · production')
    expect(uses(h)).toEqual([])
  })

  it('empty: no projects yet, and the way to add one', async () => {
    await board('start-empty')
    expect(text()).toContain('No projects on this machine yet')
    await click(btn('Add a project'))
    expect(el.querySelector('aside[aria-label="Setup steps"]')).not.toBeNull()
  })
})

describe('Start (phone)', () => {
  it('one column of rows that are their own action, the account in a line, add actions below', async () => {
    ui.narrow = true
    const h = await board('start')
    await settle()
    expect(el.querySelector('[role=toolbar] h2')?.textContent).toBe('Projects')
    expect(el.querySelector('[aria-label=Account]')).toBeNull()
    expect(text()).toContain('Signed in as ana@iii.dev · teams iii, acme')
    // "as of" before Refresh, as on the desktop.
    expect(el.querySelector('[role=toolbar]')?.textContent).toMatch(/as of \d/)
    // The rows in a panel "On this machine"; each ends with its word and a chevron, with no leading dot.
    expect(text()).toContain('On this machine')
    expect(place('registry')?.querySelector('.iii-cloud-start-word')?.textContent).toBe('Open')
    expect(place('billing')?.querySelector('.iii-cloud-start-word')?.textContent).toBe('Switch')
    expect(place('billing')?.querySelector('.iii-cloud-start-word svg')).not.toBeNull()
    expect(place('billing')?.querySelector(':scope > [data-dot]')).toBeNull()
    expect(lines('billing')[1]).toBe('~/workspaces/billing · team acme')
    expect(btn('Add a project')).toBeTruthy()
    expect(btn('New project in iii cloud')).toBeTruthy()
    await click(place('billing'))
    expect(uses(h)).toEqual([
      {
        path: '/home/you/workspaces/billing',
        cloud_project_id: 'p-billing',
        environment_id: 'e-billing-dev',
        mode: 'checkout',
      },
    ])
  })
})
