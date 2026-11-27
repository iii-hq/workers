// The shell (U1; frozen with CloudPage.tsx): the router, the step rail, the Manage sidebar, env header and tabs.
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { CloudPage } from '../CloudPage'
import { boards } from '../fixtures'
import { apiPath, base, ENV, earlierReady, signedOut } from '../fixtures/base'
import { cloudDeploy } from '../fixtures/manage'
import { DEV, FILE } from '../fixtures/setup'
import { hubOf } from '../hub'
import type { ChangeEvent, Status } from '../types'
import { fakeHost } from './host'
import { board, btn, click, cloudAnswers, ctx, el, flush, mount, root, row, text, ui, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

const main = base
const dev = { id: 'e-dev', name: 'dev', production: false, branch: 'develop' }
/** Live once, with a second environment that is not selected. */
const live: Status = {
  ...main,
  project: {
    ...main.project!,
    environments: [ENV, dev],
    checkout: { ...main.project!.checkout, not_served: [{ sha: 'a'.repeat(40), message: 'Fix the migration' }] },
  },
  deploys: [earlierReady],
}
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
    expect(row('production')?.querySelector('[aria-label=Production]')).toBeNull()
    expect(row('dev')?.querySelector('[aria-label=Production]')).toBeNull()
    // The state line of each environment, from iii cloud's deployments (nothing is deployed to dev).
    expect(row('dev')?.textContent).toContain('Not deployed yet')
    expect(side()?.textContent).toContain('Folder ~/workspaces/registry')
    expect(side()?.textContent).toContain('you@example.com · Owner')
    // A new environment is made in iii cloud's console: a link built from the connection's cloud_url.
    const link = [...side()!.querySelectorAll('a')].find((a) => a.textContent?.startsWith('New environment'))
    expect(link?.getAttribute('href')).toBe('https://cloud.iii.dev/iii/registry/production/new-environment')
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
    expect(el.querySelector('section[aria-label="Live now"]')).not.toBeNull()
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
    expect(el.querySelector('section[aria-label="Live now"]')).toBeNull()
    const project = { ...live.project!, environment: { ...dev, manifest: null, url: null } }
    await fire(h, { kind: 'project', id: 'p1', terminal: true, summary: '', record: project })
    expect(path()).toBe('Projects / registry · dev')
    expect(el.querySelector('section[aria-label="Live now"]')).not.toBeNull()
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
      { at: 'setup', adding: true },
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
    expect(text()).toContain('Deployment #40 is live')
  })

  it('Deploy to production opens the check and deploy screen of the environment', async () => {
    // With nothing the check is holding the person for, the folder's commits are the next action.
    const h = await mount({ ...live, check: null }, ctx({ at: 'env', env: ENV.id }))
    await click(btn('Deploy to production'))
    expect(h.calls.find((c) => c.id === 'iii-cloud::check')?.payload).toMatchObject({ run: true })
    expect(path()).toBe('Projects / registry · production / Deploy')
    // One screen, no step rail: the header says the next thing to do.
    expect(rail('Deploy steps')).toEqual([])
    expect(el.querySelector('h2.iii-cloud-h')?.textContent).toBe('Check main for production')
  })

  it('Add a project opens the confirmation in add mode, at the folder; Cancel comes back to Start', async () => {
    const h = await mount(live, ctx({ at: 'start' }))
    await click(btn('Add a project'))
    expect(el.querySelector('aside[aria-label="Setup steps"]')).toBeNull()
    expect(path()).toBe('Projects / Add a project')
    expect(text()).toContain('Use a project already in iii cloud')
    await click(btn('Cancel'))
    expect(path()).toBe('Projects')
    expect(uses(h)).toEqual([])
  })
})

describe('no deploy pill in the path bar (no board has one; Projects says which project needs the person)', () => {
  const pills = () => [...el.querySelectorAll('[role=toolbar] .iii-cloud-pill, [role=toolbar] button[aria-busy]')]

  it.each([
    'manage-overview-deploying',
    'setup-check-deploying',
    'start-deploying',
    'manage-overview-moving',
  ] as const)('%s: the bar has the place and "as of", no deploy button', async (id) => {
    await board(id)
    expect(pills()).toEqual([])
    expect(el.querySelector('[role=toolbar]')?.textContent).not.toMatch(/deploying|needs you/i)
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

  it('a phone: the Deploying chip is on the name line, and the row has no chevron', async () => {
    ui.narrow = true
    await board('start-deploying')
    expect(el.querySelector('.iii-cloud-start-name .iii-cloud-state')?.textContent).toBe('Deploying')
    expect(el.querySelector('.iii-cloud-start-row svg:not(.iii-cloud-state svg)')).toBeNull()
  })
})

describe("every project's deploys going on (running_deploys)", () => {
  const uses = (h: ReturnType<typeof fakeHost>) =>
    h.calls.filter((c) => c.id === 'iii-cloud::project::use').map((c) => c.payload)
  const confirmLeave = async () => {
    const ok = [...el.querySelectorAll('[role=alertdialog] button')].find((b) => b.textContent?.startsWith('Open '))
    if (ok) await click(ok)
  }

  it("another project's held deployment: its row says Needs you and names the deployment", async () => {
    const b = boards['start-moving']
    const [, billing] = b.status.running_deploys ?? []
    const held = { ...billing, deploy: { ...billing.deploy, status: 'needs_action' as const } }
    await mount({ ...b.status, running_deploys: [held] }, b.context ? ctx(b.context) : {}, cloudAnswers(b.cloud ?? {}))
    expect(text()).toContain('billingNeeds youdev · deployment #8 needs your action')
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

describe('the hand-over between places, end to end', () => {
  const calls = (h: ReturnType<typeof fakeHost>, ...ids: string[]) =>
    h.calls.filter((c) => ids.includes(c.id)).map((c) => c.id)
  const settle = async () => {
    for (let i = 0; i < 5; i++) await flush()
  }

  it('Projects: Open goes to the environment and switches nothing', async () => {
    const h = await mount(live, ctx({ at: 'start' }), cloudAnswers({ [apiPath('/deployments')]: { deployments: [] } }))
    await click(btn('Open'))
    expect(path()).toBe('Projects / registry · production')
    expect(uses(h)).toEqual([])
  })

  it('first time: Confirm takes the project up, starts the check, then opens the Deploy screen', async () => {
    const b = boards['setup-existing']
    // The worker takes the project up on project::use: the status carries its environment from then on.
    let used = false
    const status = (() =>
      used
        ? {
            ...b.status,
            project: {
              ...b.status.project!,
              workspace: { mode: 'worktree' as const, path: '~/workspaces/registry-iii-cloud', branch: 'b' },
              environment: { ...DEV, manifest: FILE, url: null },
              environments: [DEV],
            },
          }
        : b.status) as unknown as Status
    const h = await mount(status, ctx(b.context), {
      ...cloudAnswers(b.cloud ?? {}),
      'iii-cloud::project::use': () => {
        used = true
        return {}
      },
      // The local run takes minutes: nothing waits for it before the Deploy screen opens.
      'iii-cloud::check': () => new Promise(() => undefined),
    })
    // An older pick (a refused Change picked another file) must not outlive this Confirm.
    hubOf(h.host).setPickedManifest(DEV.id, 'api/old-pick.yaml')
    await click(el.querySelector('button[data-variant=primary]'))
    await settle()
    expect(hubOf(h.host).pickedManifest(DEV.id)).not.toBe('api/old-pick.yaml')
    expect(calls(h, 'iii-cloud::project::use', 'iii-cloud::check')).toEqual([
      'iii-cloud::project::use',
      'iii-cloud::check',
    ])
    expect(h.calls.find((c) => c.id === 'iii-cloud::check')?.payload).toEqual({ run: true })
    expect(path()).toBe('Projects / registry · dev / Deploy')
  })

  it('the environment: Deploy to <env> starts the check, then opens the Deploy screen', async () => {
    const h = await mount({ ...live, check: null }, ctx({ at: 'env', env: ENV.id }))
    await click(btn('Deploy to production'))
    expect(calls(h, 'iii-cloud::check')).toEqual(['iii-cloud::check'])
    expect(h.calls.find((c) => c.id === 'iii-cloud::check')?.payload).toMatchObject({ run: true })
    expect(path()).toBe('Projects / registry · production / Deploy')
  })

  it('the Deploy screen: Back to <env> returns to the environment', async () => {
    await board('deploy-follow-live')
    await click(btn('Back to dev'))
    expect(path()).toBe('Projects / registry · dev')
  })

  it('one count and one source label: the environment card and the Deploy screen say the same', async () => {
    const b = boards['deploy-problems']
    const count = (s: string | null | undefined) => /^(\d+) (?:things?|values?) /.exec(s ?? '')?.[1]
    const ws = b.status.project!.workspace!
    const status = {
      ...b.status,
      project: { ...b.status.project!, workspace: { ...ws, source: 'iii-cloud/ready-dev' } },
    }
    // A first deploy: both read the same values (nothing a file can fill here), so the counts are the same list.
    const answers = {
      ...cloudAnswers(b.cloud ?? {}),
      'iii-cloud::values': {
        environment_id: status.project?.environment?.id,
        environment: 'dev',
        manifest: 'api/worker-compose-prod.yaml',
        read_from: 'workspace',
        cloud: 'read',
        save: 'ok',
        items: [],
      },
    }
    await mount(status, ctx({ at: 'deploy', env: '' }), answers)
    const onDeploy = count(el.querySelector('h2.iii-cloud-h')?.textContent)
    expect(onDeploy).toBeDefined()
    expect(text()).not.toContain('iii-cloud/ready-')
    await mount(status, ctx({ at: 'env', env: '' }, 2), answers)
    await settle()
    expect(count(text().match(/\d+ things? needs? you/)?.[0])).toBe(onDeploy)
    expect(text()).not.toContain('iii-cloud/ready-')
  })

  it('the Deploy screen: Secrets and variables, and Review of the filled names, open the secrets page', async () => {
    const b = boards['deploy-problems']
    await board('deploy-problems')
    await click(btn('Secrets and variables'))
    expect(path()).toBe('Projects / registry · dev / Secrets and variables')
    const filled = {
      environment_id: b.status.project?.environment?.id,
      environment: 'dev',
      manifest: 'api/worker-compose-prod.yaml',
      read_from: 'workspace',
      cloud: 'read',
      save: 'ok',
      items: [
        {
          name: 'FILLED',
          need: 'required',
          in_cloud: 'set',
          here: 'from_file',
          here_file: 'api/.env.test',
          suggestions: [],
        },
      ],
    }
    await mount(b.status, b.context ? ctx(b.context, 2) : {}, { 'iii-cloud::values': filled })
    await click(btn('Review'))
    expect(path()).toBe('Projects / registry · dev / Secrets and variables')
  })

  it("a failed deployment's record: Fix with your agent hands off, then opens the Deploy screen", async () => {
    const h = await board('manage-deploys-failed')
    await settle()
    expect(path()).toBe('Projects / registry · dev / Deployment #2')
    await click(btn('Fix with your agent'))
    await settle()
    expect(h.calls.find((c) => c.id === 'iii-cloud::handoff')?.payload).toMatchObject({
      kind: 'deploy_fix',
      deployment_id: 'dep-2',
    })
    expect(path()).toBe('Projects / registry · dev / Deploy')
  })

  it('chat cards open routes that exist: the check, the secrets page, a deployment followed, a deployment recorded', async () => {
    const env = ENV.id
    const routes: [unknown, RegExp][] = [
      [{ at: 'deploy', env }, /production \/ Deploy$/],
      [{ at: 'secrets', env }, /production \/ Secrets and variables$/],
      [{ at: 'deploy', env, deploy: earlierReady.id }, /production \/ Deploy$/],
      [{ at: 'env', env, deploy: earlierReady.id }, /production \/ Deployment #40$/],
    ]
    for (const [i, [route, shown]] of routes.entries()) {
      await mount(live, ctx(route, i + 1), cloudAnswers({ [apiPath('/deployments')]: { deployments: [] } }))
      await settle()
      expect(path()).toMatch(shown)
      expect(text()).not.toContain('Not in a project on this machine')
    }
  })
})
