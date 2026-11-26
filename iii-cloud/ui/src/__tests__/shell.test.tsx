// The shell (U1; frozen with CloudPage.tsx): the router, the step rail, the Manage sidebar, env header and tabs.
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { CloudPage } from '../CloudPage'
import { boards } from '../fixtures'
import { apiPath, base, ENV, earlierReady, signedOut } from '../fixtures/base'
import { cloudDeploy } from '../fixtures/manage'
import type { ChangeEvent, Status } from '../types'
import { fakeHost } from './host'
import { board, btn, click, cloudAnswers, ctx, el, flush, mount, root, row, text, ui, usePage } from './util'

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
const side = () => el.querySelector('aside[aria-label="registry"]')
/** The environment the sidebar marks as the page. */
const current = () => side()?.querySelector('[aria-current=page]')?.textContent
const uses = (h: ReturnType<typeof fakeHost>) => h.calls.filter((c) => c.id === 'iii-cloud::project::use')

describe('Manage', () => {
  it("opens on Start; Open goes to the project's environment: the sidebar lists the environments, one is the page", async () => {
    const h = await mount(
      live,
      {},
      cloudAnswers({
        [apiPath('/deployments')]: { deployments: [] },
        [apiPath('/deployments', 'e-dev')]: { deployments: [] },
      }),
    )
    expect(path()).toBe('Projects')
    expect(el.querySelector('aside')).toBeNull()
    await click(btn('Open'))
    expect(uses(h)).toEqual([])
    expect(description()).toBe('you@example.com · team iii (Owner)')
    expect(side()).not.toBeNull()
    expect(side()?.textContent).toContain('registry')
    expect(side()?.textContent).toContain('iii-hq/registry · team iii')
    expect(current()).toContain('production')
    expect(row('production')?.querySelector('[aria-label=Production]')).not.toBeNull()
    expect(row('dev')?.querySelector('[aria-label=Production]')).toBeNull()
    // The state line of each environment, from iii cloud's deployments (nothing is deployed to dev).
    expect(row('dev')?.textContent).toContain('Not deployed yet')
    expect(side()?.textContent).toContain('Folder ~/workspaces/registry')
    expect(side()?.textContent).toContain('you@example.com · Owner')
    // A new environment is made in iii cloud's console: a link built from the connection's cloud_url.
    const link = [...side()!.querySelectorAll('a')].find((a) => a.textContent?.startsWith('New environment'))
    expect(link?.getAttribute('href')).toBe('https://cloud.iii.dev/iii/registry')
  })

  it('an environment page: the path names it, no tabs, Refresh reads the status again, Secrets and variables is a page', async () => {
    const live40 = cloudDeploy(40, earlierReady.commit.sha, '', 'ready', 60, null, { current: true })
    const h = await mount(
      live,
      ctx({ at: 'home' }),
      cloudAnswers({ [apiPath('/deployments')]: { deployments: [live40] } }),
    )
    expect(current()).toContain('production')
    expect(path()).toBe('Projects / registry · production')
    expect(el.querySelector('[role=toolbar]')?.textContent).toContain('Live')
    expect(el.querySelector('[role=toolbar]')?.textContent).toMatch(/as of \d{1,2}:\d{2}/)
    // No environment tabs on a wide pane (the old Overview/Deploys/Settings/Checks bar is gone too).
    expect(tabs()).not.toContain('production')
    expect(tabs()).not.toContain('Overview')
    expect(el.querySelector('section[aria-label=Serving]')).not.toBeNull()
    await click(btn('Refresh'))
    expect(h.calls.at(-1)).toEqual({ id: 'iii-cloud::status', payload: { refresh: true } })
    await click(btn('Secrets and variables'))
    expect(path()).toBe('Projects / registry · production / Secrets and variables')
    expect(el.querySelector('aside')).toBeNull()
    // The path's environment goes back to it, and Projects goes to Start.
    await click(btn('registry · production'))
    expect(path()).toBe('Projects / registry · production')
    await click(btn('Projects'))
    expect(path()).toBe('Projects')
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
    expect(path()).toBe('Projects / registry · dev')
    expect(el.querySelector('section[aria-label=Serving]')).not.toBeNull()
    expect(uses(h)).toHaveLength(1)
  })

  it('a stored env route restored on mount whose env is not selected opens the selected one, with no project::use', async () => {
    ui.panes['iii-cloud:route:t'] = { at: 'env', env: 'e-dev' }
    const h = await mount(live)
    expect(current()).toContain('production')
    expect(uses(h)).toEqual([])
  })

  it('a pane whose env another pane moved the selection away from opens the selected one, not a skeleton', async () => {
    const h = await mount(live, ctx({ at: 'env', env: ENV.id }))
    expect(path()).toBe('Projects / registry · production')
    const project = { ...live.project!, environment: { ...dev, manifest: null, url: null } }
    await fire(h, { kind: 'project', id: 'p1', terminal: true, summary: '', record: project })
    expect(path()).toBe('Projects / registry · dev')
    expect(el.querySelector('.iii-cloud-skeleton')).toBeNull()
    expect(uses(h)).toEqual([])
  })

  it('a narrow pane: the environments are the tabs under the path, with no sidebar', async () => {
    ui.narrow = true
    await mount(live, ctx({ at: 'env', env: ENV.id }))
    expect(el.querySelector('aside')).toBeNull()
    expect(tabs()).toEqual(['production', 'dev'])
    expect(selectedTab()).toBe('production')
    // The path bar's back goes to Projects.
    expect(btn('Back to Projects')).toBeTruthy()
  })

  it('the header description is who is signed in, in every area', async () => {
    const places: unknown[] = [
      { at: 'start' },
      { at: 'home' },
      { at: 'env', env: ENV.id },
      { at: 'deploy', env: ENV.id },
      { at: 'secrets', env: ENV.id },
      { at: 'setup' },
      { at: 'setup', step: 'confirm', adding: true },
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

  it('a switch the worker refuses goes back to the selected environment and says why', async () => {
    const gone = Object.assign(new Error('gone'), { code: 'ENVIRONMENT_DELETED' })
    await mount(live, ctx({ at: 'env', env: 'e-dev' }), { 'iii-cloud::project::use': gone })
    expect(text()).toContain('That environment no longer exists in iii cloud.')
    expect(current()).toContain('production')
  })

  it('an environment in no project of this machine is an empty state with the way to Projects', async () => {
    const h = await mount(live, ctx({ at: 'env', env: 'nope' }))
    expect(text()).toContain('Not in a project on this machine')
    expect(uses(h)).toEqual([])
    await click(btn('Projects'))
    expect(path()).toBe('Projects')
  })

  it('a new panels.open re-navigates; old contexts land on the environment, its secrets or a deployment', async () => {
    const h = fakeHost(live)
    const at = async (id: number, context: unknown) => {
      await act(async () => root.render(<CloudPage host={h.host} panelSide="left" tabId="t" {...ctx(context, id)} />))
      await flush()
    }
    await at(1, { at: 'home' })
    expect(path()).toBe('Projects / registry · production')
    await at(1, { at: 'home' })
    expect(path()).toBe('Projects / registry · production')
    await at(2, { tab: 'secrets' })
    expect(path()).toBe('Projects / registry · production / Secrets and variables')
    expect(text()).toContain('Secrets and variables')
    await at(3, { tab: 'deploys', deployId: earlierReady.id })
    expect(path()).toBe(`Projects / registry · production / Deployment #${earlierReady.number}`)
    expect(text()).toContain('Deployment #40 · 1a2b3c4')
  })

  it('Deploy to production opens the check and deploy screen of the environment', async () => {
    await mount(live, ctx({ at: 'env', env: ENV.id }))
    await click(btn('Deploy to production'))
    expect(path()).toBe('Projects / registry · production / Deploy')
    expect(rail('Deploy steps')).toEqual(['current', 'todo', 'todo', 'todo'])
    expect(text()).toContain('What goes to production?')
  })

  it('Add a project opens the confirmation in add mode, at the folder; Cancel comes back to Start', async () => {
    const h = await mount(live, ctx({ at: 'start' }))
    await click(btn('Add a project'))
    expect(el.querySelector('aside[aria-label="Setup steps"]')).toBeNull()
    expect(path()).toBe('Projects / Add a project')
    expect(text()).toContain('Pick the folder of your project.')
    await click(btn('Cancel'))
    expect(path()).toBe('Projects')
    expect(uses(h)).toEqual([])
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
    expect(pills().map((p) => p.textContent)).toEqual(['Deployment #6 needs you'])
    expect(pills()[0].querySelector('[data-dot]')?.getAttribute('data-dot')).toBe('warn')
    ui.narrow = true
    await mount(waiting, b.context ? ctx(b.context) : {}, cloudAnswers(b.cloud ?? {}))
    expect(pills().map((p) => p.textContent)).toEqual(['#6 · needs you'])
    expect(pills()[0].getAttribute('aria-label')).toBe('Deployment #6 needs you')
  })

  it('a phone: "#6 · deploying", the whole sentence in its aria-label', async () => {
    ui.narrow = true
    await board('setup-check-deploying')
    const p = pills()[0]
    expect(p?.textContent).toBe('#6 · deploying')
    expect(p?.getAttribute('aria-label')).toMatch(/^Deploying #6 to dev · \d+ min$/)
  })

  it('more than two: "+N" opens the first one\'s environment', async () => {
    const b = boards['manage-overview-deploying']
    const [d] = b.status.deploys
    const many = { ...b.status, deploys: [d, { ...d, id: 'd7', number: 7 }, { ...d, id: 'd8', number: 8 }] }
    await mount(many, b.context ? ctx(b.context) : {}, cloudAnswers(b.cloud ?? {}))
    expect(pills().map((p) => p.textContent?.slice(0, 12))).toEqual(['Deploying #6', 'Deploying #7', '+1'])
    await click(pills()[2])
    expect(path()).toBe('Projects / registry · dev')
  })

  it('the Manage sidebar: the env deploying has a live dot and "deploying #6"', async () => {
    await board('manage-overview-deploying')
    const env = row('devdeploying #6')
    expect(env?.querySelector('[data-dot]')?.getAttribute('data-pulse')).toBe('')
  })
})

describe('Start: the project in use with a deploy going on', () => {
  it('the row says Deploying and what runs; its one action is Open, to the environment', async () => {
    await board('start-deploying')
    expect(text()).toMatch(/registryDeployingproduction · deployment #6 is running · \d+ min so far/)
    expect(btn('Follow deploy')).toBeNull()
    expect(btn('Open')?.getAttribute('data-variant')).toBe('secondary')
    await click(btn('Open'))
    expect(path()).toBe('Projects / registry · production')
  })

  it('a phone: the row ends with the Deploying pill', async () => {
    ui.narrow = true
    await board('start-deploying')
    expect(el.querySelector('.iii-cloud-start-word')?.textContent).toBe('Deploying')
  })
})

describe("every project's deploys going on (running_deploys)", () => {
  const pills = () => [...el.querySelectorAll('[role=toolbar] .iii-cloud-pill')]
  const uses = (h: ReturnType<typeof fakeHost>) =>
    h.calls.filter((c) => c.id === 'iii-cloud::project::use').map((c) => c.payload)
  const confirmLeave = async () => {
    const ok = [...el.querySelectorAll('[role=alertdialog] button')].find((b) => b.textContent?.startsWith('Open '))
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

  it('Start: every row with a deploy going on says it; Open on another project takes it up first', async () => {
    const h = await board('start-moving')
    expect(text()).toMatch(/registryDeployingproduction · deployment #6 is running · \d+ min so far/)
    expect(text()).toMatch(/billingDeployingdev · deployment #8 is running · \d+ min so far/)
    await click(el.querySelector('button[aria-label^="Open billing"]'))
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
