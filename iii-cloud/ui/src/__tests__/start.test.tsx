// Projects: the account, the projects on this machine, "Open" on every row and its guard.
import { describe, expect, it, vi } from 'vitest'
import { base, ENV, request, signedOut, TEAM } from '../fixtures/base'
import { running6 } from '../fixtures/deploy'
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
/** A row by its project's name, its lines, its pill, and its one button. */
const place = (name: string) =>
  rows().find((r) => r.querySelector('.iii-cloud-start-name')?.firstChild?.textContent === name)
const lines = (name: string) =>
  [...(place(name)?.querySelectorAll('.iii-cloud-start-lines > span') ?? [])].map((s) => s.textContent)
const pill = (name: string) => place(name)?.querySelector('.iii-cloud-state')?.textContent
const openOf = (name: string) => place(name)?.querySelector('button[aria-label^="Open "]') as HTMLButtonElement | null
const uses = (h: { calls: Call[] }) => h.calls.filter((c) => c.id === 'iii-cloud::project::use').map((c) => c.payload)
const reads = (h: { calls: Call[] }) =>
  h.calls.filter((c) => c.id === 'iii-cloud::api').map((c) => String(c.payload?.path))
const path = () => el.querySelector('[role=toolbar] nav')?.textContent
const showStart = async (status: Status = startStatus, answers = {}) => {
  const h = await mount(status, ctx({ at: 'start' }), { ...cloudAnswers(startCloud), ...answers })
  await settle()
  return h
}

describe('Projects (desktop)', () => {
  it('the path, who is signed in, the count, one add button, the rows in order (in use, recent, this folder’s other projects)', async () => {
    const h = await board('start')
    await settle()
    expect(path()).toBe('Projects')
    const bar = el.querySelector('[role=toolbar]')?.textContent
    expect(bar).toContain('on this machine')
    expect(bar).toContain('ana@iii.dev · teams iii, acme')
    expect(el.querySelector('[aria-label=Account]')).toBeNull()
    // The card says how many, with the one add button at its end.
    expect(el.querySelector('[role=region] .iii-cloud-panel-head')?.textContent).toContain('5 projects')
    expect([...el.querySelectorAll('.iii-cloud-panel-head button')].map((b) => b.textContent)).toEqual([
      'Add a project',
    ])
    expect(names()).toEqual(['registry', 'billing', 'registry-3', 'old', 'registry-eu'])
    // Line 2: the folder (mono) and the team.
    expect(lines('billing')[1]).toBe('~/workspaces/billing · team acme')
    expect(place('registry')?.querySelector('.iii-cloud-start-path .iii-cloud-mono')?.textContent).toBe(
      '~/workspaces/registry',
    )
    expect(text()).toContain("Opening a project doesn't stop the others.")
    // Every row has one action, Open: no Switch, no In use, nothing about a worker moving.
    for (const name of ['registry', 'billing', 'registry-3', 'registry-eu']) {
      expect([...(place(name)?.querySelectorAll('button') ?? [])].map((b) => b.textContent)).toEqual(['Open'])
    }
    expect(text()).not.toMatch(/Switch|In use|this worker|the worker/)
    // The pill is the state of the environment.
    expect(pill('registry')).toBe('Live')
    expect(pill('billing')).toBe('Deploying')
    expect(pill('registry-eu')).toBe('Failed')
    expect(pill('registry-3')).toBe('Not deployed yet')
    expect(pill('old')).toBe('Deleted')
    // Deleted in iii cloud: until when it can be restored, the way to iii cloud, and no Open.
    expect(lines('old')[0]).toBe('restorable until 3 Dec')
    expect(place('old')?.querySelector('a')?.textContent).toBe('Open in iii cloud')
    expect(place('old')?.querySelector('a')?.getAttribute('href')).toMatch(
      /^https:\/\/cloud\.iii\.dev\/iii\/deletions$/,
    )
    expect(openOf('old')).toBeNull()
    // Open takes another project up with project::use, then lands on its environment.
    expect(openOf('billing')?.getAttribute('aria-label')).toBe('Open billing in ~/workspaces/billing')
    await click(openOf('billing'))
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

  it('something waits on the person in the project in use: its pill says Needs you, its line what waits', async () => {
    await showStart({ ...startStatus, secrets: [request('SESSION_SECRET', 'ask', 'waiting', '')] })
    expect(pill('registry')).toBe('Needs you')
    expect(lines('registry')[0]).toContain('1 secret is waiting')
  })

  it('a deploy going and something waiting: the row says what waits, not only that it deploys', async () => {
    const env = startStatus.project!.environment!
    const going = { ...running6, environment_id: env.id, from_console: false }
    await showStart({
      ...startStatus,
      secrets: [request('SESSION_SECRET', 'ask', 'waiting', '')],
      deploys: [going, ...startStatus.deploys],
    })
    expect(lines('registry')[0]).toContain('is running')
    expect(lines('registry')[0]).toContain('1 secret is waiting')
    expect(pill('registry')).toBe('Needs you')
  })

  it('at most nine rows', async () => {
    const many: RecentProject[] = Array.from({ length: 12 }, (_, i) => ({ ...billing, path: `/home/you/w/p${i}` }))
    await showStart({ ...startStatus, recent_projects: many })
    expect(rows()).toHaveLength(9)
  })

  it('first line from iii cloud: what the env serves, what runs, what failed; nothing invented', async () => {
    const h = await board('start')
    await settle()
    expect(lines('registry')[0]).toMatch(/^production · serves 1a2b3c4 · deployment #40 · (\d\d:\d\d|\d+ \w{3})$/)
    // The sha is mono; the time is the clock, never "2h ago" (the environment page says the same).
    expect(place('registry')?.querySelector('.iii-cloud-start-state .iii-cloud-mono')?.textContent).toBe('1a2b3c4')
    expect(lines('registry')[0]).not.toMatch(/ago/)
    expect(lines('billing')[0]).toBe('dev · deployment #8 is running')
    expect(lines('registry-eu')[0]).toMatch(/^eu · deployment #3 failed( · still serves #\d+)?$/)
    // registry-3's environment is gone: its row reads production.
    expect(lines('registry-3')[0]).toBe('production · not deployed yet')
    expect(reads(h)).toContain(`/teams/${ACME.id}/projects/p-billing/environments`)
    expect(reads(h)).toContain(`/teams/${ACME.id}/environments/e-billing-dev/deployments`)
    // The project in use reads only its environment's deployments: its environments are in the status.
    expect(reads(h)).not.toContain(`/teams/${TEAM.id}/projects/p1/environments`)
  })

  it('only the folder and names are known: the row says what exists and how to see the rest', async () => {
    const lone: RecentProject = { ...billing, team_id: undefined, team_name: undefined }
    await showStart({ ...startStatus, recent_projects: [lone] })
    expect(lines('billing')[0]).toBe("dev · open it to see what's live")
    expect(pill('billing')).toBeUndefined()
  })

  it('until iii cloud lists the environments, Open is off and busy', async () => {
    const answers = cloudAnswers(startCloud)
    const api = (q: Record<string, unknown>) =>
      String(q.path).includes('p-billing') ? new Promise(() => undefined) : answers['iii-cloud::api'](q)
    const h = await showStart(startStatus, { 'iii-cloud::api': api })
    expect(openOf('billing')?.disabled).toBe(true)
    expect(openOf('billing')?.getAttribute('aria-busy')).toBe('true')
    expect(openOf('registry-3')?.disabled).toBe(false)
    await click(openOf('billing'))
    expect(uses(h)).toEqual([])
  })

  it('an environment iii cloud no longer lists: Open sends no environment_id', async () => {
    const h = await board('start')
    await settle()
    await click(openOf('registry-3'))
    expect(uses(h)).toEqual([{ path: '/home/you/workspaces/registry-3', cloud_project_id: 'p-r3', mode: 'worktree' }])
  })

  it('the default environment taken instead: the page says so', async () => {
    const answer = () => ({
      ...base.project,
      environment: { ...ENV, manifest: null, url: null },
      fell_back_from: 'e-billing-dev',
    })
    await showStart(startStatus, { 'iii-cloud::project::use': answer })
    await click(openOf('billing'))
    expect(text()).toContain('dev no longer exists; showing production')
  })

  it('deleted, in a team the person is no longer in: no link to the console (never another team’s)', async () => {
    const left = startStatus.recent_projects!.map((r) =>
      r.cloud_project_id === 'p-old' ? { ...r, team_id: 't-left' } : r,
    )
    await showStart({ ...startStatus, recent_projects: left })
    expect(lines('old')[0]).toBe('deleted in iii cloud')
    expect(place('old')?.querySelector('a')).toBeNull()
  })

  it('a read that fails is said once for the list, with Try again; the row waits and Open stays; the read works after', async () => {
    let broken = true
    const fail = () => ({ status: 500, body: { code: 'CLOUD_ERROR', message: '' } })
    const answers = cloudAnswers(startCloud)
    const api = (q: Record<string, unknown>) =>
      broken && String(q.path).includes('p-billing') ? fail() : answers['iii-cloud::api'](q)
    await showStart(startStatus, { 'iii-cloud::api': api })
    expect(lines('billing')[0]).toBe("What is live isn't known yet")
    expect(text().match(/Couldn't read/g)).toHaveLength(1)
    expect(text()).toContain('iii cloud had a problem. Try again in a moment.')
    expect(openOf('billing')?.disabled).toBe(false)
    broken = false
    await click(btn('Try again'))
    await settle()
    expect(text()).not.toContain("Couldn't read")
    expect(lines('billing')[0]).not.toBe("What is live isn't known yet")
  })

  it('something waits on the person: leaving asks first, and Stay sends nothing', async () => {
    const waiting = { ...startStatus, secrets: [request('SESSION_SECRET', 'ask', 'waiting', '')] }
    const h = await showStart(waiting)
    await click(openOf('billing'))
    const dialog = el.querySelector('[role=alertdialog]')
    expect(dialog?.textContent).toContain('Leave registry · production while it waits for you?')
    expect(dialog?.textContent).toContain('1 secret is waiting. It keeps waiting; open it again to answer.')
    expect(btn('Open billing')).toBeTruthy()
    await click(btn('Stay'))
    expect(el.querySelector('[role=alertdialog]')).toBeNull()
    expect(uses(h)).toEqual([])
    await click(openOf('billing'))
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
    await click(openOf('billing'))
    expect(el.querySelector('[role=alertdialog]')).not.toBeNull()
    down = true
    await click(btn('Refresh'))
    expect(text()).toContain("iii cloud isn't answering")
    await click(btn('Open billing'))
    expect(el.querySelector('[role=alertdialog]')).toBeNull()
    expect(uses(h)).toEqual([])
  })

  it('an open that fails: "Couldn\'t open billing" with the reason, the page stays on Projects', async () => {
    const gone = Object.assign(new Error('billing no longer exists in iii cloud.'), { code: 'PROJECT_DELETED' })
    await showStart(startStatus, { 'iii-cloud::project::use': gone })
    await click(openOf('billing'))
    expect(path()).toBe('Projects')
    const alert = el.querySelector('[role=status][data-variant=alert]')?.textContent
    expect(alert).toContain("Couldn't open billing")
    expect(alert).toContain('billing no longer exists in iii cloud.')
    expect(alert).not.toMatch(/switch/i)
    expect(openOf('billing')?.disabled).toBe(false)
  })

  it('signed out: no reads, the way to sign in, no live state, Open off except for the project in use', async () => {
    const h = await showStart({ ...startStatus, connection: signedOut })
    expect(reads(h)).toEqual([])
    expect(text()).toContain('Not signed in to iii cloud')
    expect(lines('billing')[0]).toBe("Sign in to see what's live")
    expect(pill('billing')).toBeUndefined()
    expect(openOf('billing')?.disabled).toBe(true)
    await click(btn('Sign in with your browser'))
    expect(h.calls.filter((c) => c.id === 'iii-cloud::connect').map((c) => c.payload)).toEqual([{ method: 'browser' }])
  })

  it('an API key: one row read through the key’s environment, no add button, and the browser sign-in offered', async () => {
    const key = { secret: 'IIICLOUD_API_KEY', team_id: TEAM.id, environment_id: ENV.id }
    const status = {
      ...startStatus,
      connection: { ...startStatus.connection, method: 'api_key' as const, api_key: key },
      project: { ...startStatus.project!, team: null, cloud_project: null },
    }
    const h = await showStart(status)
    expect(names()).toEqual(['iii-hq/registry'])
    expect(reads(h)).toEqual([`/teams/${TEAM.id}/environments/${ENV.id}/deployments`])
    expect(text()).toContain('IIICLOUD_API_KEY · reaches one environment')
    expect(btn('Add a project')).toBeNull()
    expect(btn('Sign in with your browser')).toBeTruthy()
  })

  it('Add a project opens the confirmation in add mode, with nothing selected; Open on the one in use goes home with no call', async () => {
    const h = await board('start')
    await settle()
    await click(btn('Add a project'))
    expect(path()).toBe('Projects / Add a project')
    expect(uses(h)).toEqual([])
    await click(btn('Projects'))
    await click(openOf('registry'))
    expect(path()).toBe('Projects / registry · production')
    expect(uses(h)).toEqual([])
  })

  it('empty: says what this is and how to add a project', async () => {
    await board('start-empty')
    expect(text()).toContain('No projects on this machine yet')
    expect(text()).toContain('Add the folder of a project.')
    await click(btn('Add a project'))
    expect(path()).toBe('Put this project on iii cloud')
    expect(el.querySelector('[data-directory-picker]')).not.toBeNull()
  })
})

describe('Projects (phone)', () => {
  it('one card of rows that open whole, the chip on the name line and no chevron, one add button below', async () => {
    ui.narrow = true
    const h = await board('start')
    await settle()
    expect(el.querySelector('[role=toolbar] h2')?.textContent).toBe('Projects')
    expect(text()).toContain('Signed in as ana@iii.dev · teams iii, acme')
    expect(el.querySelector('[role=toolbar]')?.textContent).toMatch(/as of \d/)
    expect(el.querySelector('.iii-cloud-start-card [aria-label="Projects on this machine"]')).not.toBeNull()
    expect(place('registry')?.querySelector('.iii-cloud-start-name .iii-cloud-state')?.textContent).toBe('Live')
    expect(place('billing')?.querySelector('svg')).toBeNull()
    // The folder is mono here too.
    expect(place('billing')?.querySelector('.iii-cloud-start-path .iii-cloud-mono')?.textContent).toBe(
      '~/workspaces/billing',
    )
    // No button inside a row: the row is the action.
    expect(place('billing')?.querySelector('button button')).toBeNull()
    expect(lines('billing')[1]).toBe('~/workspaces/billing · team acme')
    expect(text()).toContain(
      'The whole row opens the project. A check, a deploy and your agent keep going on this machine, and the row says what they are doing.',
    )
    expect([...el.querySelectorAll('.iii-cloud-start-adds button')].map((b) => b.textContent)).toEqual([
      'Add a project',
    ])
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
