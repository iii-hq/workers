// The shell (U1; frozen with CloudPage.tsx): the router, the step rail, the Manage sidebar, env header and tabs.
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { CloudPage } from '../CloudPage'
import { boards } from '../fixtures'
import { apiPath, base, ENV, earlierReady, signedOut } from '../fixtures/base'
import { cloudDeploy } from '../fixtures/manage'
import type { ChangeEvent, Status } from '../types'
import { fakeHost } from './host'
import { board, btn, click, cloudAnswers, ctx, el, flush, mount, root, row, text, type, ui, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

const main = base
const dev = { id: 'e-dev', name: 'dev', production: false, branch: 'develop' }
/** Live once, with a second environment that is not selected. */
const live: Status = { ...main, project: { ...main.project!, environments: [ENV, dev] }, deploys: [earlierReady] }
const rail = (label: string) =>
  [...el.querySelectorAll(`aside[aria-label="${label}"] [role=row]`)].map((r) => r.getAttribute('data-step'))
const tabs = () => [...el.querySelectorAll('[role=tab]')].map((t) => t.textContent)
const selectedTab = () => el.querySelector('[role=tab][aria-selected=true]')?.textContent
const fire = (h: ReturnType<typeof fakeHost>, ev: ChangeEvent) =>
  act(async () => h.handlers.get('iii::iii-cloud-ui::events::console-test')?.(ev))
const description = () => el.querySelector('header [data-slot=description]')?.textContent
const path = () => el.querySelector('[role=toolbar] nav')?.textContent
/** The sidebar's first row: the project, current at its home. */
const projectRow = () => row('registryiii-hq/registry')
const atHome = () => projectRow()?.getAttribute('aria-current') === 'page'
const uses = (h: ReturnType<typeof fakeHost>) => h.calls.filter((c) => c.id === 'iii-cloud::project::use')

describe('the step rail', () => {
  it('the step shown is aria-current; done steps go back, the step the status is at too; steps ahead are off', async () => {
    await board('setup-target')
    expect(rail('Setup steps')).toEqual(['done', 'done', 'current', 'todo', 'todo', 'todo', 'todo'])
    expect(el.querySelector('aside [aria-current=step]')?.textContent).toContain('Where it runs')
    expect(text()).toContain('Where should it run?')
    await click(row('Start and sign in'))
    expect(text()).toContain('Signed in')
    expect(text()).not.toContain('Where should it run?')
    expect(rail('Setup steps')).toEqual(['current', 'done', 'reached', 'todo', 'todo', 'todo', 'todo'])
    expect(el.querySelector('aside [aria-current=step]')?.textContent).toContain('Start and sign in')
    // A step ahead is a plain row: legible, and a click on it goes nowhere.
    const ahead = row('4')
    await click(ahead)
    expect(text()).toContain('Signed in')
    await click(row('3'))
    expect(text()).toContain('Where should it run?')
  })
})

describe('Manage', () => {
  it('opens on Start; Open goes home: the sidebar lists the environments, home is the current page', async () => {
    const h = await mount(live)
    expect(path()).toBe('Projects')
    expect(el.querySelector('aside')).toBeNull()
    await click(btn('Open'))
    expect(uses(h)).toEqual([])
    expect(description()).toBe('you@example.com · team iii (Owner)')
    const side = el.querySelector('aside[aria-label="registry"]')
    expect(side).not.toBeNull()
    expect(atHome()).toBe(true)
    expect(projectRow()?.hasAttribute('data-selected')).toBe(true)
    expect(row('All environments')).toBeUndefined()
    expect(row('production')?.querySelector('[aria-label=Production]')).not.toBeNull()
    expect(row('dev')?.querySelector('[aria-label=Production]')).toBeNull()
    expect(side?.textContent).toContain('Folder ~/workspaces/registry')
    expect(side?.textContent).toContain('Signed in · Owner on team iii')
  })

  it('an environment page: the header names it, four line tabs, Refresh reads the status again', async () => {
    const current = cloudDeploy(40, earlierReady.commit.sha, '', 'ready', 60, null, { current: true })
    const h = await mount(
      live,
      ctx({ at: 'home' }),
      cloudAnswers({ [apiPath('/deployments')]: { deployments: [current] } }),
    )
    await click(row('production'))
    expect(row('production')?.getAttribute('aria-current')).toBe('page')
    expect(projectRow()?.hasAttribute('aria-current')).toBe(false)
    expect(path()).toBe('Projects / registry / production')
    expect(el.querySelector('[role=toolbar]')?.textContent).toContain('Live')
    expect(el.querySelector('[role=toolbar]')?.textContent).toMatch(/as of \d{1,2}:\d{2}/)
    expect(tabs()).toEqual(['Overview', 'Deploys', 'Settings', 'Checks'])
    expect(selectedTab()).toBe('Overview')
    expect(el.querySelector('section[aria-label=Serving]')).not.toBeNull()
    await click(btn('Checks'))
    expect(selectedTab()).toBe('Checks')
    await click(btn('Refresh'))
    expect(h.calls.at(-1)).toEqual({ id: 'iii-cloud::status', payload: { refresh: true } })
    // The path's project goes home, and Projects goes to Start.
    await click(btn('registry'))
    expect(atHome()).toBe(true)
    await click(btn('Projects'))
    expect(path()).toBe('Projects')
    await click(btn('Open'))
    // Checks opens the check itself in Deploy from this machine.
    await click(row('production'))
    await click(btn('Checks'))
    await click(btn('Open the check'))
    expect(text()).toContain('iii cloud stops at the first thing it cannot accept')
  })

  it('an environment that is not selected is selected first, once, with a skeleton until it is', async () => {
    const h = await mount(live, ctx({ at: 'home' }))
    await click(row('dev'))
    expect(uses(h).map((c) => c.payload)).toEqual([
      { path: '/home/you/workspaces/registry', environment_id: 'e-dev', mode: 'worktree' },
    ])
    expect(row('dev')?.getAttribute('aria-current')).toBe('page')
    expect(el.querySelector('section[aria-label=Serving]')).toBeNull()
    const project = { ...live.project!, environment: { ...dev, manifest: null, url: null } }
    await fire(h, { kind: 'project', id: 'p1', terminal: true, summary: '', record: project })
    expect(path()).toBe('Projects / registry / dev')
    expect(el.querySelector('section[aria-label=Serving]')).not.toBeNull()
    expect(uses(h)).toHaveLength(1)
  })

  it('a stored env route restored on mount whose env is not selected opens home, with no project::use', async () => {
    ui.panes['iii-cloud:route:t'] = { at: 'env', env: 'e-dev', tab: 'deploys' }
    const h = await mount(live)
    expect(atHome()).toBe(true)
    expect(uses(h)).toEqual([])
  })

  it('a pane whose env another pane moved the selection away from goes home, not to a skeleton', async () => {
    const h = await mount(live, ctx({ at: 'env', env: ENV.id }))
    expect(path()).toBe('Projects / registry / production')
    const project = { ...live.project!, environment: { ...dev, manifest: null, url: null } }
    await fire(h, { kind: 'project', id: 'p1', terminal: true, summary: '', record: project })
    expect(atHome()).toBe(true)
    expect(el.querySelector('.iii-cloud-skeleton')).toBeNull()
    expect(uses(h)).toEqual([])
  })

  it('the header description is who is signed in, in every area', async () => {
    const places: unknown[] = [
      { at: 'start' },
      { at: 'home' },
      { at: 'env', env: ENV.id },
      { at: 'deploy', env: ENV.id },
      { at: 'new-env' },
      { at: 'setup' },
      { at: 'setup', step: 'code', adding: true },
    ]
    for (const [i, at] of places.entries()) {
      await mount(live, ctx(at, i + 1))
      expect(description()).toBe('you@example.com · team iii (Owner)')
    }
  })

  it('signed out with a project in use: Start, with the way to sign in', async () => {
    const h = await mount({ ...live, connection: signedOut }, ctx({ at: 'home' }))
    expect(path()).toBe('Projects')
    expect(description()).toBe('Not connected')
    await click(btn('Sign in with your browser'))
    expect(h.calls.filter((c) => c.id === 'iii-cloud::connect')).toEqual([
      { id: 'iii-cloud::connect', payload: { method: 'browser' } },
    ])
  })

  it('a switch the worker refuses goes back home and says why', async () => {
    const gone = Object.assign(new Error('gone'), { code: 'ENVIRONMENT_DELETED' })
    await mount(live, ctx({ at: 'env', env: 'e-dev', tab: 'deploys' }), { 'iii-cloud::project::use': gone })
    expect(text()).toContain('That environment no longer exists in iii cloud.')
    expect(atHome()).toBe(true)
  })

  it('an environment in no project of this machine is an empty state with the way to Projects', async () => {
    const h = await mount(live, ctx({ at: 'env', env: 'nope' }))
    expect(text()).toContain('Not in a project on this machine')
    expect(uses(h)).toEqual([])
    await click(btn('Projects'))
    expect(path()).toBe('Projects')
  })

  it('a new panels.open re-navigates; old contexts land on their tab of the selected environment', async () => {
    const h = fakeHost(live)
    const at = async (id: number, context: unknown) => {
      await act(async () => root.render(<CloudPage host={h.host} panelSide="left" tabId="t" {...ctx(context, id)} />))
      await flush()
    }
    await at(1, { at: 'home' })
    await click(row('production'))
    expect(selectedTab()).toBe('Overview')
    await at(1, { at: 'home' })
    expect(selectedTab()).toBe('Overview')
    await at(2, { tab: 'secrets' })
    expect(selectedTab()).toBe('Settings')
    expect(text()).toContain('Secrets and variables')
    await at(3, { tab: 'deploys', deployId: earlierReady.id })
    expect(selectedTab()).toBe('Deploys')
    expect(text()).toContain('Deploy #40 · 1a2b3c4')
  })

  it('Deploy from this machine opens the deploy steps of the environment', async () => {
    await mount(live, ctx({ at: 'env', env: ENV.id }))
    await click(btn('Deploy from this machine'))
    expect(path()).toBe('Projects / registry / production / Deploy')
    expect(rail('Deploy steps')).toEqual(['current', 'todo', 'todo', 'todo'])
    expect(text()).toContain('What goes to production?')
  })

  it('Add a project… opens setup at the folder in add mode; Cancel comes back to Start', async () => {
    const h = await mount(live, ctx({ at: 'start' }))
    await click(btn('Add a project…'))
    expect(el.querySelector('aside[aria-label="Setup steps"]')).not.toBeNull()
    expect(path()).toBe('Projects / Add a project')
    expect(text()).toContain('Which code goes to the cloud?')
    await click(btn('Cancel'))
    expect(path()).toBe('Projects')
    expect(uses(h)).toEqual([])
  })

  it('New environment: the form, then the deploy steps of the environment it made', async () => {
    let status = live
    const made = { id: 'e-new', name: 'qa', production: false, branch: 'main' }
    const h = fakeHost(() => status, {
      'iii-cloud::project::create_environment': () => {
        const p = status.project!
        status = {
          ...status,
          project: {
            ...p,
            environment: { ...made, manifest: null, url: null },
            environments: [...p.environments, made],
          },
        }
        return { environment_id: 'e-new', project_id: 'p1' }
      },
    })
    await act(async () => root.render(<CloudPage host={h.host} panelSide="right" tabId="t" {...ctx({ at: 'home' })} />))
    await flush()
    await click(row('New environment'))
    expect(row('New environment')?.getAttribute('aria-current')).toBe('page')
    expect((el.querySelector('#iii-cloud-ne-branch') as HTMLInputElement).value).toBe('main')
    await type(el.querySelector('#iii-cloud-ne-name'), 'qa')
    await click(btn('Create qa'))
    await flush()
    expect(path()).toBe('Projects / registry / qa / Deploy')
    expect(el.querySelector('aside[aria-label="Deploy steps"]')).not.toBeNull()
    expect(h.calls.some((c) => c.id === 'iii-cloud::project::use')).toBe(false)
  })
})

describe('DeployPill: the way back to a deploy going on', () => {
  const pills = () => [...el.querySelectorAll('[role=toolbar] .iii-cloud-pill')]
  const steps = (label: string) => el.querySelector(`aside[aria-label="${label}"] [aria-current=step]`)?.textContent

  it.each([
    'manage-overview-deploying',
    'setup-check-deploying',
    'start-deploying',
  ] as const)('%s: "Deploying #6 to <env> · N min", a live dot, in the path bar', async (id) => {
    await board(id)
    const env = boards[id].status.project?.environment?.name
    expect(pills().map((p) => p.textContent)).toEqual([expect.stringMatching(`^Deploying #6 to ${env} · \\d+ min$`)])
    expect(pills()[0].querySelector('[data-dot]')?.getAttribute('data-pulse')).toBe('')
  })

  it('the click goes to its follow page, where it is no longer shown', async () => {
    await board('manage-overview-deploying')
    await click(pills()[0])
    expect(steps('Deploy steps')).toContain('Live')
    expect(pills()).toEqual([])
  })

  it('never on its own follow page', async () => {
    await board('deploy-follow-running')
    expect(pills()).toEqual([])
  })

  it('a deploy waiting on the person: the warn dot and "Deploy #6 needs you"; a phone: "#6 · needs you"', async () => {
    const b = boards['manage-overview-deploying']
    const waiting = {
      ...b.status,
      deploys: b.status.deploys.map((d, i) => (i ? d : { ...d, status: 'needs_action' as const })),
    }
    await mount(waiting, b.context ? ctx(b.context) : {}, cloudAnswers(b.cloud ?? {}))
    expect(pills().map((p) => p.textContent)).toEqual(['Deploy #6 needs you'])
    expect(pills()[0].querySelector('[data-dot]')?.getAttribute('data-dot')).toBe('warn')
    ui.narrow = true
    await mount(waiting, b.context ? ctx(b.context) : {}, cloudAnswers(b.cloud ?? {}))
    expect(pills().map((p) => p.textContent)).toEqual(['#6 · needs you'])
    expect(pills()[0].getAttribute('aria-label')).toBe('Deploy #6 needs you')
  })

  it('a phone: "#6 · deploying", the whole sentence in its aria-label', async () => {
    ui.narrow = true
    await board('setup-check-deploying')
    const p = pills()[0]
    expect(p?.textContent).toBe('#6 · deploying')
    expect(p?.getAttribute('aria-label')).toMatch(/^Deploying #6 to dev · \d+ min$/)
  })

  it('more than two: "+N" opens the first one\'s Deploys tab', async () => {
    const b = boards['manage-overview-deploying']
    const [d] = b.status.deploys
    const many = { ...b.status, deploys: [d, { ...d, id: 'd7', number: 7 }, { ...d, id: 'd8', number: 8 }] }
    await mount(many, b.context ? ctx(b.context) : {}, cloudAnswers(b.cloud ?? {}))
    expect(pills().map((p) => p.textContent?.slice(0, 12))).toEqual(['Deploying #6', 'Deploying #7', '+1'])
    await click(pills()[2])
    expect(selectedTab()).toBe('Deploys')
  })

  it('the setup rail: step 6 says "deploying #6" and shows that deploy', async () => {
    await board('setup-check-deploying')
    const ship = [...el.querySelectorAll('aside[aria-label="Setup steps"] [role=row]')].find((r) =>
      r.textContent?.includes('Review and deploy'),
    )
    expect(ship?.textContent).toContain('Review and deploydeploying #6')
    await click(ship ?? null)
    // route.ts derivedStep: a deploy going on puts setup at step 6, which shows it as it goes.
    expect(steps('Setup steps')).toContain('Review and deploy')
    expect(el.querySelector('h2.iii-cloud-setup-title')?.textContent).toMatch(/^Deploying \w{7} to dev$/)
    expect(btn('Cancel the deploy')).toBeTruthy()
  })

  it('the Manage sidebar: the env deploying has a live dot and "deploying #6"', async () => {
    await board('manage-overview-deploying')
    const env = row('devdeploying #6')
    expect(env?.querySelector('[data-dot]')?.getAttribute('data-pulse')).toBe('')
  })
})

describe('Start: the project in use with a deploy going on', () => {
  it('"deploying #6 to production · N min"; Follow deploy (primary) goes to it, Open is a ghost beside it', async () => {
    await board('start-deploying')
    expect(text()).toMatch(/registryIn usedeploying #6 to production · \d+ min/)
    expect(btn('Follow deploy')?.getAttribute('data-variant')).toBe('primary')
    expect(btn('Open')?.getAttribute('data-variant')).toBe('ghost')
    await click(btn('Follow deploy'))
    expect(el.querySelector('aside[aria-label="Deploy steps"] [aria-current=step]')?.textContent).toContain('Live')
  })

  it('a phone: the row says Follow', async () => {
    ui.narrow = true
    await board('start-deploying')
    expect(el.querySelector('.iii-cloud-start-word')?.textContent).toBe('Follow')
  })
})

describe("every project's deploys going on (running_deploys)", () => {
  const pills = () => [...el.querySelectorAll('[role=toolbar] .iii-cloud-pill')]
  const uses = (h: ReturnType<typeof fakeHost>) =>
    h.calls.filter((c) => c.id === 'iii-cloud::project::use').map((c) => c.payload)
  const confirmLeave = async () => {
    const ok = [...el.querySelectorAll('[role=alertdialog] button')].find((b) => b.textContent?.startsWith('Switch to'))
    if (ok) await click(ok)
  }

  it('the pill: another environment of this project, then another project with its name', async () => {
    await board('manage-overview-moving')
    expect(pills().map((p) => p.textContent)).toEqual([
      expect.stringMatching(/^Deploying #7 to production · \d+ min$/),
      expect.stringMatching(/^billing · deploying #8 to dev · \d+ min$/),
    ])
  })

  it('another environment of this project: its follow route, selecting that environment, no project switch', async () => {
    const h = await board('manage-overview-moving')
    await click(pills()[0])
    await confirmLeave()
    // CloudPage selects production in the same folder: the same project.
    expect(uses(h)).toEqual([{ path: '/home/you/workspaces/registry', environment_id: ENV.id, mode: 'worktree' }])
  })

  it('another project: the switch Start uses (project::use with its folder and environment), then its follow page', async () => {
    const h = await board('manage-overview-moving')
    await click(pills()[1])
    await confirmLeave()
    await flush()
    expect(uses(h)).toEqual([
      // No mode: this status lists no recent billing folder to take it from; project::use keeps its own.
      { path: '/home/you/workspaces/billing', cloud_project_id: 'p-billing', environment_id: 'e-billing-dev' },
    ])
    expect(path()).toContain('Deploy')
  })

  it('a phone: more than one reads "2 deploying", to Projects; one of another project\'s reads "billing #8 · deploying"', async () => {
    ui.narrow = true
    await board('manage-overview-moving')
    expect(pills().map((p) => p.textContent)).toEqual(['2 deploying'])
    await click(pills()[0])
    expect(el.querySelector('[role=toolbar] h2')?.textContent).toBe('Projects')
    ui.narrow = true
    const b = boards['manage-overview-moving']
    const only = { ...b.status, running_deploys: b.status.running_deploys?.slice(1) }
    await mount(only, b.context ? ctx(b.context) : {}, cloudAnswers(b.cloud ?? {}))
    const p = pills()[0]
    expect(p?.textContent).toBe('billing #8 · deploying')
    expect(p?.getAttribute('aria-label')).toMatch(/^billing · deploying #8 to dev · \d+ min$/)
  })

  it('a deploy the worker never knew the origin of: "#9 · deploying", no name; not this project\'s: a link to iii cloud', async () => {
    const b = boards['manage-overview-moving']
    const [, billing] = b.status.running_deploys ?? []
    const unknown = {
      deploy: {
        ...billing.deploy,
        id: 'dep-x9',
        number: 9,
        environment_id: 'e-unknown',
        console_url: 'https://cloud.iii.dev/x/9',
      },
      cloud_project_id: null,
      project_name: null,
      repository: null,
      environment_name: null,
      path: null,
    }
    const h = await mount(
      { ...b.status, running_deploys: [unknown] },
      b.context ? ctx(b.context) : {},
      cloudAnswers(b.cloud ?? {}),
    )
    const p = pills()[0]
    expect(p?.textContent).toMatch(/^Deploying #9 · \d+ min$/)
    expect(p?.tagName).toBe('A')
    expect(p?.getAttribute('href')).toBe('https://cloud.iii.dev/x/9')
    expect(uses(h)).toEqual([])
    ui.narrow = true
    await mount(
      { ...b.status, running_deploys: [unknown] },
      b.context ? ctx(b.context) : {},
      cloudAnswers(b.cloud ?? {}),
    )
    expect(pills()[0]?.textContent).toBe('#9 · deploying')
  })

  it("without running_deploys (an older worker): the selected environment's deploys", async () => {
    const b = boards['manage-overview-deploying']
    expect(b.status.running_deploys).toBeUndefined()
    await board('manage-overview-deploying')
    expect(pills().map((p) => p.textContent)).toEqual([expect.stringMatching(/^Deploying #6 to dev · \d+ min$/)])
  })

  it('Start: every row with a deploy going on says it and follows it; another project switches first', async () => {
    const h = await board('start-moving')
    expect(text()).toMatch(/registryIn usedeploying #6 to production · \d+ min/)
    expect(text()).toMatch(/billing.*deploying #8 to dev · \d+ min/)
    const follows = [...el.querySelectorAll('button')].filter((b) => b.textContent === 'Follow deploy')
    expect(follows).toHaveLength(2)
    const trailing = follows[1].parentElement
    expect(
      [...(trailing?.querySelectorAll('button') ?? [])].map((b) => [b.textContent, b.getAttribute('data-variant')]),
    ).toEqual([
      ['Switch', 'ghost'],
      ['Follow deploy', 'primary'],
    ])
    await click(follows[1])
    await confirmLeave()
    await flush()
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
