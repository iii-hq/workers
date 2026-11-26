// Setup (U3): start and sign in, the code, where it runs, review and deploy, live (Onb1–Onb3, Onb6, Onb7).
// Step 4 is the shared CheckStage: its own suite covers it.
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { CloudPage } from '../CloudPage'
import { boards, cardRecords } from '../fixtures'
import { agent, apiPath, base, COMMIT, DEPLOY_BASE, ENV, pr, project, ready } from '../fixtures/base'
import { TEST_ENV } from '../fixtures/check'
import { DEV, deploying, FILE, registry, shipStatus } from '../fixtures/setup'
import { rcValues } from '../fixtures/values'
import { plainMessage } from '../model'
import type { Status } from '../types'
import { type Call, fakeHost } from './host'
import {
  board,
  btn,
  click,
  cloudAnswers,
  ctx,
  el,
  flush,
  lastCall,
  mount,
  onClose,
  root,
  row,
  text,
  type,
  usePage,
} from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

const radios = (group: string) => [...el.querySelectorAll(`[role=radiogroup][aria-label="${group}"] [role=radio]`)]
const radio = (group: string, label: string) => radios(group).find((r) => r.textContent?.startsWith(label))
const checked = (group: string) =>
  radios(group)
    .find((r) => r.getAttribute('aria-checked') === 'true')
    ?.textContent?.trim()
const of = (h: { calls: Call[] }, id: string) => h.calls.filter((c) => c.id === id).map((c) => c.payload)
/** The page's own calls, without the status and cloud reads. */
const sent = (h: { calls: Call[] }) =>
  h.calls.map((c) => c.id).filter((id) => id !== 'iii-cloud::status' && id !== 'iii-cloud::api')
const facts = () => [...el.querySelectorAll('.iii-cloud-setup-fact')].map((f) => f.textContent)
const toCode = ctx({ at: 'setup', step: 'code' })
/** Signed in, the folder found, no environment chosen yet, and no other repositories. */
const unset: Status = { ...base, project: project(false), check: null }

describe('P1: every step has its forward action top right in its header, and none below', () => {
  const fwd = () => [...el.querySelectorAll('.iii-cloud-stephead-fwd button')].map((b) => b.textContent)
  const back = () => el.querySelector('.iii-cloud-stephead-back')?.textContent
  const count = (label: string) => [...el.querySelectorAll('button')].filter((b) => b.textContent === label).length
  it.each([
    ['setup-code', 'Continue', 'Back'],
    ['setup-target', 'Continue', undefined],
    ['setup-values', 'Save 12 values and check', 'Check without saving'],
    ['setup-ship', 'Deploy 3c4d2e0 to dev', undefined],
  ] as const)('%s: %s', async (id, label, link) => {
    await board(id)
    expect(fwd()).toEqual([label])
    expect(back()).toBe(link)
    expect(count(label)).toBe(1)
  })

  it('the note under the title says what the action does', async () => {
    await board('setup-target')
    expect(el.querySelector('.iii-cloud-stephead-reason')?.textContent).toBe(
      'Next: the values your app reads. Nothing is deployed.',
    )
  })
})

describe('1 start and sign in (Onb1)', () => {
  it('waiting: the five moments, the code to type, the wait, the device page without the code', async () => {
    await board('setup-signin')
    expect(text()).toContain('Put your project on iii cloud')
    expect(text()).toContain('You step in five times.')
    expect(el.querySelectorAll('[aria-label="Where you step in"] li')).toHaveLength(5)
    expect(el.querySelector('[aria-label="Where you step in"]')?.textContent).toContain(
      'Give the values your app reads',
    )
    expect(text()).toContain('Sign in to iii cloud')
    expect(text()).toContain('WXYZ-1234')
    expect(text()).toMatch(/Waiting for your approval · the code expires in 9:\d\d/)
    const link = el.querySelector('a[href="https://cloud.iii.dev/device"]')
    expect(link?.getAttribute('target')).toBe('_blank')
    expect(link?.textContent).toContain('Open cloud.iii.dev/device')
    expect(el.innerHTML).not.toMatch(/href="[^"]*WXYZ/)
    expect(el.querySelector('header [data-slot=description]')?.textContent).toBe('Not connected')
    await click(btn('Close'))
    expect(onClose).toHaveBeenCalled()
  })

  it('signed out: the browser button calls connect; the API key waits behind a disclosure', async () => {
    const h = await mount({ ...boards['setup-signin'].status, connection: { ...cardRecords.signedOut } })
    expect(text()).not.toContain('Waiting for your approval')
    await click(btn('Sign in with the browser'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::connect', payload: { method: 'browser' } })
    expect(el.querySelector('#iii-cloud-secret-name')).toBeNull()
    await click(btn("Can't use a browser here? Use an API key"))
    expect((el.querySelector('#iii-cloud-secret-name') as HTMLInputElement).value).toBe('IIICLOUD_API_KEY')
    expect(text()).toContain("It can't create projects or environments, so setup needs the browser at least once.")
  })

  it('the key goes to the vault with secrets::set, then connect carries only its name', async () => {
    const h = await mount({ ...boards['setup-signin'].status, connection: { ...cardRecords.signedOut } })
    await click(btn("Can't use a browser here? Use an API key"))
    await type(el.querySelector('input[type="password"]'), 'k-123')
    await click(btn('Save key'))
    expect(sent(h).slice(-2)).toEqual(['secrets::set', 'iii-cloud::connect'])
    expect(lastCall(h.calls)?.payload).toEqual({ method: 'api_key', secret: 'IIICLOUD_API_KEY' })
    expect(h.calls.filter((c) => JSON.stringify(c).includes('k-123')).map((c) => c.id)).toEqual(['secrets::set'])
  })

  it('BROWSER_SIGN_IN_UNAVAILABLE: a warn panel with Try again; the key opens with its name focused', async () => {
    const err = Object.assign(new Error('This iii cloud does not accept worker sign-in yet; use an API key.'), {
      code: 'BROWSER_SIGN_IN_UNAVAILABLE',
    })
    await mount(
      { ...boards['setup-signin'].status, connection: { ...cardRecords.signedOut } },
      {},
      { 'iii-cloud::connect': err },
    )
    await click(btn('Sign in with the browser'))
    const panel = el.querySelector('[role=status][data-variant=warn]')?.textContent ?? ''
    expect(panel).toContain("Browser sign-in isn't available on this iii cloud yet")
    expect(panel).toContain('Use an API key below.')
    expect(panel).not.toContain('BROWSER_SIGN_IN_UNAVAILABLE')
    expect(btn('Sign in with the browser')).toBeNull()
    expect(btn('Try again')?.getAttribute('data-variant')).toBe('secondary')
    expect(document.activeElement?.id).toBe('iii-cloud-secret-name')
  })

  it('signed in, step 1 says so and goes on; SIGN_IN_REQUIRED brings setup back to it', async () => {
    await board('setup-target', ctx({ at: 'setup', step: 'signin' }))
    expect(text()).toContain('you@example.com · team iii (Owner)')
    await click(btn('Continue'))
    expect(text()).toContain('Where should it run?')

    const s = base
    const p = { ...s.check!.problems[0], id: 'sir', code: 'SIGN_IN_REQUIRED', owner: 'person' as const }
    const h = await mount({ ...s, check: { ...s.check!, problems: [p] } }, ctx({ at: 'setup' }, 2))
    expect(el.querySelector('[aria-label="Setup steps"] [aria-current=step]')?.textContent).toContain(
      'Start and sign in',
    )
    await click(btn('Sign in again'))
    expect(of(h, 'iii-cloud::connect')).toEqual([{ method: 'browser' }])
  })
})

describe('2 your code (Onb2)', () => {
  it('the rail marks Your code current while it shows, though the status is at Where it runs', async () => {
    await board('setup-target', ctx({ at: 'setup', step: 'code' }))
    expect(text()).toContain('Which code goes to the cloud?')
    const rail = [...el.querySelectorAll('aside[aria-label="Setup steps"] [role=row]')]
    expect(rail.map((r) => r.getAttribute('data-step'))).toEqual([
      'done',
      'current',
      'reached',
      'todo',
      'todo',
      'todo',
      'todo',
    ])
    expect(el.querySelector('[aria-label="Setup steps"] [aria-current=step]')?.textContent).toContain('Your code')
    // The rail's title names the project, as Deploy's names the environment.
    expect(el.querySelector('aside[aria-label="Setup steps"] .iii-cloud-dep-railhead')?.textContent).toBe(
      'Set up registry',
    )
    // Where it runs is not done (nothing is in use yet): a number, and the way back to it.
    expect(rail[2].querySelector('.iii-cloud-rail-mark')?.textContent).toBe('3')
    await click(rail[2])
    expect(text()).toContain('Where should it run?')
  })

  it('what the worker found and the app files; a new folder resolves and the page stays here', async () => {
    const h = await board('setup-code')
    expect(text()).toContain('Which code goes to the cloud?')
    expect(facts().slice(0, 2)).toEqual([
      'Repositoryiii-hq/registry · on GitHub · default branch main',
      'You are onmain',
    ])
    expect(radios('App file the cloud runs').map((r) => r.textContent)).toEqual([
      'api/worker-compose-prod.yamlRecommended · has a cloud section · 9 workers · as production',
      'api/worker-compose.yamlno cloud section · 9 workers · as development',
    ])
    expect(checked('App file the cloud runs')).toMatch(/^api\/worker-compose-prod\.yaml/)
    expect(text()).toContain("starting from GitHub's main. You have 4 files changed here that aren't on GitHub.")
    await type(el.querySelector('[data-directory-picker]'), '/home/you/workspaces/other')
    expect(lastCall(h.calls)).toEqual({
      id: 'iii-cloud::project::resolve',
      payload: { path: '/home/you/workspaces/other' },
    })
    expect(text()).toContain('Which code goes to the cloud?')
  })

  it('no project yet: the chat folder resolves on arrival; Continue waits for it', async () => {
    const h = await mount(
      { ...boards['setup-code'].status, project: null },
      { ...toCode, workingDir: '/home/you/workspaces/registry' },
    )
    expect(of(h, 'iii-cloud::project::resolve')).toEqual([{ path: '/home/you/workspaces/registry' }])
    expect(btn('Continue')?.hasAttribute('disabled')).toBe(true)
  })

  it('only the branch the worker made is ever named, never a guessed one', async () => {
    await mount(unset, toCode)
    expect(text()).not.toContain('iii-cloud/ready-production')
  })

  it('bring my changed files: bring_changes in project::use; the check waits for the values', async () => {
    const h = await board('setup-target', toCode)
    expect(text()).toContain("You have 2 files changed here that aren't on GitHub.")
    await click(el.querySelector('.iii-cloud-setup-note input[type=checkbox]'))
    await click(btn('Continue'))
    expect(text()).toContain('Where should it run?')
    await click(btn('Continue'))
    expect(sent(h).slice(-1)).toEqual(['iii-cloud::project::use'])
    expect(of(h, 'iii-cloud::project::use')).toEqual([
      {
        path: '/home/you/workspaces/registry',
        cloud_project_id: 'p1',
        environment_id: ENV.id,
        mode: 'worktree',
        bring_changes: true,
      },
    ])
    expect(of(h, 'iii-cloud::check')).toEqual([])
  })

  it('letting the agent edit this folder asks first, in plain words', async () => {
    const h = await board('setup-target', toCode)
    await click(btn('Let it edit this folder instead…'))
    expect(el.querySelector('[role=alertdialog]')?.textContent).toContain('Let your agent edit your own files?')
    await click(btn('Edit my own files'))
    expect(text()).toContain('Your agent will change the files in this folder directly, with no separate copy.')
    await click(btn('Continue'))
    await click(btn('Continue'))
    expect(of(h, 'iii-cloud::project::use')[0]).toMatchObject({ mode: 'checkout' })
    expect(of(h, 'iii-cloud::project::use')[0]).not.toHaveProperty('bring_changes')
  })

  it('an app file picked here is the one Values reads and the check plans; one iii cloud does not plan is off', async () => {
    const compose_files = [
      { path: 'api/worker-compose-prod.yaml', choosable: true, suggested_environment: null, issue: null },
      { path: 'api/worker-compose-next.yaml', choosable: true, suggested_environment: null, issue: null },
      {
        path: 'api/workers-compose-dev.yaml',
        choosable: false,
        suggested_environment: null,
        issue: 'NOT_WORKER_COMPOSE',
      },
    ]
    const h = await mount({ ...base, compose_files }, toCode)
    const file = (p: string) => radio('App file the cloud runs', p)
    expect(file('api/workers-compose-dev.yaml')?.getAttribute('aria-disabled')).toBe('true')
    expect(file('api/workers-compose-dev.yaml')?.textContent).toContain('not a worker-compose*.yaml')
    expect(file('api/worker-compose-prod.yaml')?.getAttribute('aria-checked')).toBe('true')
    await click(file('api/worker-compose-next.yaml'))
    expect(file('api/worker-compose-next.yaml')?.getAttribute('aria-checked')).toBe('true')
    await click(btn('Continue'))
    await click(btn('Continue'))
    expect(of(h, 'iii-cloud::check')).toEqual([])
    expect(el.querySelector('.iii-cloud-setup')?.getAttribute('data-step')).toBe('values')
    expect(of(h, 'iii-cloud::values')).toEqual([{ manifest: 'api/worker-compose-next.yaml' }])
    await click(btn('Check'))
    expect(of(h, 'iii-cloud::check')).toEqual([{ run: true, manifest: 'api/worker-compose-next.yaml' }])
  })

  it('changing a project in use: prefilled, and Continue goes back to where the page was', async () => {
    const h = await mount(base, ctx({ at: 'setup' }))
    await click(row('Your code'))
    expect((el.querySelector('[data-directory-picker]') as HTMLInputElement).value).toBe(
      '/home/you/workspaces/registry',
    )
    expect(btn('Back')).toBeNull()
    await click(btn('Continue'))
    await click(btn('Continue'))
    expect(of(h, 'iii-cloud::project::use')[0]).toMatchObject({
      path: '/home/you/workspaces/registry',
      mode: 'worktree',
    })
    expect(text()).not.toContain('Where should it run?')
    expect(el.querySelector('.iii-cloud-setup')?.getAttribute('data-step')).toBe('values')
  })
})

describe('3 where it runs (Onb3)', () => {
  it("the repository's projects, a new one, the other repositories by name; deleted ones hidden", async () => {
    const s = boards['setup-target'].status
    const gone = { id: 'p2', name: 'old', slug: 'old', team: registry.team, deleted: true, environments: [] }
    const withOld = {
      ...registry,
      environments: [...registry.environments!, { id: 'e-old', name: 'old-env', branch: 'x', deleted: true }],
    }
    await mount(
      { ...s, project: { ...s.project!, cloud_projects: [withOld, gone] } },
      {},
      cloudAnswers(boards['setup-target'].cloud!),
    )
    expect(text()).toContain('Where should it run?')
    expect(radios('Project').map((r) => r.textContent)).toEqual([
      'registryBuilt from iii-hq/registry · 2 environmentsthis repository',
      'New projectImports iii-hq/registry and creates its first environment',
    ])
    expect(checked('Project')).toMatch(/^registry/)
    expect(text()).not.toContain('old-env')
    expect(text()).not.toContain('hello-iii')
    await click(btn('3 projects in team iii use other repositories'))
    expect(text()).toContain('hello-iii, docs-site, learn-iii. Open their folder to manage them.')
    expect(radio('Project', 'hello-iii')).toBeUndefined()
  })

  it('with no other repositories, nothing extra is shown', async () => {
    await mount(unset)
    expect(text()).not.toContain('other repositories')
  })

  it('environments: the branch, how each deploys, and what iii cloud has live', async () => {
    await board('setup-target')
    await flush()
    expect(radios('Environment').map((r) => r.textContent)).toEqual([
      'productionFollows main · deploys when its pull request is mergedNothing live yet',
      'devFollows develop · deploys after the check, from this machineLast deploy failed',
      'New environmentPick a name and the branch it follows; settings copy from dev',
    ])
    const b = boards['setup-target']
    await mount(
      b.status,
      ctx(b.context, 2),
      cloudAnswers({ ...b.cloud, [apiPath('/deployments')]: { deployments: [{ status: 'ready', current: true }] } }),
    )
    await flush()
    expect(radio('Environment', 'production')?.textContent).toMatch(/Live$/)
  })

  it('Continue sends the chosen project and environment; other teams are named and not read', async () => {
    const s = boards['setup-target'].status
    const eu = { id: 'e-eu', name: 'production', production: true, branch: 'release', manifest: 'eu/compose.yaml' }
    const partners = { id: 'p2', name: 'registry-eu', slug: 'registry-eu', team: { id: 't2', name: 'partners' } }
    const h = await mount(
      { ...s, project: { ...s.project!, cloud_projects: [registry, { ...partners, environments: [eu] }] } },
      {},
      cloudAnswers(boards['setup-target'].cloud!),
    )
    expect(radio('Project', 'registry-eu')?.textContent).toContain('team partners')
    await click(radio('Project', 'registry-eu'))
    expect(radios('Environment').map((r) => r.textContent?.split('Follows')[0])).toEqual([
      'production',
      'New environmentPick a name and the branch it follows; settings copy from production',
    ])
    await click(btn('Continue'))
    expect(of(h, 'iii-cloud::project::use')[0]).toMatchObject({ cloud_project_id: 'p2', environment_id: 'e-eu' })
    const read = h.calls.filter((c) => c.id === 'iii-cloud::api').map((c) => String(c.payload?.path))
    expect(read.some((p) => p.includes('e-eu'))).toBe(false)
  })

  it('PROJECT_DELETED: a panel with "Choose another", which picks another project', async () => {
    const s = boards['setup-target'].status
    const second = {
      id: 'p3',
      name: 'registry-eu',
      slug: 'eu',
      team: registry.team,
      environments: [{ id: 'e3', name: 'production', production: true, branch: 'main', manifest: null }],
    }
    // The worker names the project itself; an empty message keeps the page's own sentence.
    const err = Object.assign(new Error('registry no longer exists in iii cloud.'), { code: 'PROJECT_DELETED' })
    await mount(
      { ...s, project: { ...s.project!, cloud_projects: [registry, second] } },
      {},
      { 'iii-cloud::project::use': err },
    )
    await click(btn('Continue'))
    expect(text()).toContain('registry no longer exists in iii cloud.')
    expect(plainMessage({ code: 'PROJECT_DELETED', message: '' })).toBe('That project no longer exists in iii cloud.')
    await click(btn('Choose another'))
    expect(text()).not.toContain('no longer exists')
    expect(checked('Project')).toMatch(/^registry-eu/)
  })

  it('no project yet: the create form, Continue says why it is off, then the made ids are used', async () => {
    const made = {
      ...registry,
      id: 'p9',
      environments: [
        { ...ENV, manifest: 'api/worker-compose-prod.yaml' },
        {
          id: 'e-extra',
          name: 'staging',
          production: false,
          branch: 'develop',
          manifest: 'api/worker-compose-prod.yaml',
        },
      ],
    }
    const s = boards['setup-code'].status
    const h = await mount(
      {
        ...s,
        connection: {
          ...s.connection,
          teams: [...s.connection.teams, { id: 't2', name: 'partners', slug: 'p', role: 'Owner' }],
        },
      },
      ctx({ at: 'setup', step: 'target' }),
      { 'iii-cloud::project::create': { cloud_projects: [made], project_id: 'p9', environment_id: 'e-extra' } },
    )
    expect(checked('Project')).toMatch(/^New project/)
    expect(el.querySelector('[aria-label="New project in iii cloud"]')).not.toBeNull()
    expect(btn('Continue')?.hasAttribute('disabled')).toBe(true)
    expect(text()).toContain('Create the project first.')
    const team = el.querySelector('select[aria-label="Team"]') as HTMLSelectElement
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value')?.set?.call(team, 't2')
      team.dispatchEvent(new Event('change', { bubbles: true }))
    })
    await click(btn('Create project'))
    expect(of(h, 'iii-cloud::project::create')).toEqual([
      {
        path: '/home/you/workspaces/registry',
        team_id: 't2',
        manifest: 'api/worker-compose-prod.yaml',
        environment_name: 'production',
        production: true,
      },
    ])
    expect(el.querySelector('[aria-label="New project in iii cloud"]')).toBeNull()
    expect(checked('Project')).toMatch(/^registry/)
    expect(checked('Environment')).toMatch(/^staging/)
    await click(btn('Continue'))
    expect(of(h, 'iii-cloud::project::use')[0]).toMatchObject({ cloud_project_id: 'p9', environment_id: 'e-extra' })
  })

  it('with projects, "New project" opens the form in place of the environments; Cancel closes it', async () => {
    await board('setup-target')
    await click(radio('Project', 'New project'))
    expect(el.querySelector('[aria-label="New project in iii cloud"]')).not.toBeNull()
    expect(radios('Environment')).toHaveLength(0)
    await click(btn('Cancel'))
    expect(el.querySelector('[aria-label="New project in iii cloud"]')).toBeNull()
    expect(radios('Environment')).toHaveLength(3)
  })

  it('a new environment: the branch says how it deploys, then the one made is selected and used', async () => {
    const stg = { id: 'e-stg', name: 'staging', production: false, branch: 'iii-cloud/ready-staging', manifest: FILE }
    const h = await board(
      'setup-target',
      {},
      {
        'iii-cloud::project::create_environment': {
          cloud_projects: [{ ...registry, environments: [...registry.environments!, stg] }],
        },
      },
    )
    expect(radio('Environment', 'New environment')?.textContent).toContain('settings copy from dev')
    await click(radio('Environment', 'New environment'))
    expect((el.querySelector('#iii-cloud-ne-branch') as HTMLInputElement).value).toBe('main')
    expect(text()).toContain('It deploys when a pull request into main is merged.')
    expect(text()).toContain('Create the environment first.')
    await type(el.querySelector('#iii-cloud-ne-name'), 'staging')
    await type(el.querySelector('#iii-cloud-ne-branch'), 'iii-cloud/ready-staging')
    expect(text()).toContain('This machine deploys it after the check.')
    await click(btn('Create staging'))
    expect(of(h, 'iii-cloud::project::create_environment')).toEqual([
      { cloud_project_id: 'p1', name: 'staging', branch: 'iii-cloud/ready-staging', manifest: FILE, copy_from: DEV.id },
    ])
    expect(el.querySelector('[aria-label="New environment in iii cloud"]')).toBeNull()
    expect(checked('Environment')).toMatch(/^staging/)
    await click(btn('Continue'))
    expect(of(h, 'iii-cloud::project::use')[0]).toMatchObject({ environment_id: 'e-stg' })
  })

  it('creating an environment: errors read as plain text and the form stays; Creating… while it runs', async () => {
    const err = Object.assign(new Error('ENV_NAME_TAKEN: dup'), { code: 'ENV_NAME_TAKEN' })
    await board('setup-target', {}, { 'iii-cloud::project::create_environment': err })
    await click(radio('Environment', 'New environment'))
    await type(el.querySelector('#iii-cloud-ne-name'), 'production')
    await click(btn('Create production'))
    expect(text()).toContain('An environment with that name already exists in this project. Pick another name.')
    expect(text()).not.toContain('ENV_NAME_TAKEN')
    expect(btn('Create production')?.hasAttribute('disabled')).toBe(false)

    let release: (v: unknown) => void = () => undefined
    const gate = new Promise((r) => {
      release = r
    })
    await board('setup-target', ctx({ at: 'setup' }, 2), { 'iii-cloud::project::create_environment': () => gate })
    await click(radio('Environment', 'New environment'))
    await type(el.querySelector('#iii-cloud-ne-name'), 'qa')
    await click(btn('Create qa'))
    expect(btn('Creating…')?.hasAttribute('disabled')).toBe(true)
    await act(async () => release({ cloud_projects: [registry] }))
  })

  it('iii cloud cannot see the repository: the agent imports it with the app file picked in step 2', async () => {
    const s = boards['setup-code'].status
    const manifests = [
      ...s.project!.importable!.manifests,
      { path: 'api/worker-compose-old.yaml', choosable: false, suggested_environment: null, issue: 'No lock file yet' },
    ]
    const status: Status = {
      ...s,
      project: { ...s.project!, github_access: false, importable: { ...s.project!.importable!, manifests } },
    }
    const h = fakeHost(status, { 'iii-cloud::handoff': { id: 'h', marker: '[m]' } })
    await act(async () => root.render(<CloudPage host={h.host} panelSide="right" tabId="t" {...toCode} />))
    await flush()
    const files = radios('App file the cloud runs')
    expect(files.map((r) => r.getAttribute('aria-disabled'))).toEqual([null, null, 'true'])
    expect(files[2].textContent).toContain('No lock file yet')
    await click(radio('App file the cloud runs', 'api/worker-compose.yaml'))
    await click(btn('Continue'))
    expect(text()).toContain("iii cloud can't see iii-hq/registry on GitHub yet")
    expect(el.querySelector('[aria-label="New project in iii cloud"]')).toBeNull()
    await click(btn('Ask the agent to import it'))
    expect(h.compose.mock.calls[0][0].text).toContain(
      'Import iii-hq/registry with api/worker-compose.yaml as development.',
    )
    // The project being set up is the one in use: no candidate flag.
    expect(of(h, 'iii-cloud::handoff')[0]).not.toHaveProperty('candidate')
  })
})

/** The same check on production, which deploys through a pull request. */
const prodShip = (over: Partial<Status> = {}): Status => ({
  ...shipStatus,
  project: { ...shipStatus.project!, environment: { ...ENV, manifest: 'api/worker-compose-prod.yaml', url: null } },
  check: { ...shipStatus.check!, environment_id: ENV.id },
  handoff_brief: null,
  ...over,
})

describe('4 values (Onb3b)', () => {
  const allSet = { ...rcValues, items: rcValues.items.map((i) => ({ ...i, in_cloud: 'set' as const })) }
  const savedAll = (p: Record<string, unknown>) => ({
    environment_id: ENV.id,
    results: (p.items as { name: string }[]).map((x) => ({ name: x.name, ok: true })),
  })

  it('Values reads the file picked in step 2; Save N values and check saves first, then checks that file', async () => {
    const compose_files = [
      { path: 'api/worker-compose-prod.yaml', choosable: true, suggested_environment: null, issue: null },
      { path: 'api/worker-compose-next.yaml', choosable: true, suggested_environment: null, issue: null },
    ]
    const values = {
      ...rcValues,
      environment_id: ENV.id,
      environment: ENV.name,
      manifest: 'api/worker-compose-next.yaml',
    }
    const h = await mount({ ...base, compose_files }, toCode, {
      'iii-cloud::values': values,
      'iii-cloud::values::save': savedAll,
    })
    await click(radio('App file the cloud runs', 'api/worker-compose-next.yaml'))
    await click(btn('Continue'))
    await click(btn('Continue'))
    expect(text()).toContain('Give the values your app reads')
    expect(of(h, 'iii-cloud::values')).toEqual([{ manifest: 'api/worker-compose-next.yaml' }])
    await click(btn('Save 12 values and check'))
    await flush()
    expect(
      sent(h)
        .filter((id) => id !== 'iii-cloud::values')
        .slice(-2),
    ).toEqual(['iii-cloud::values::save', 'iii-cloud::check'])
    expect(of(h, 'iii-cloud::values::save')[0]).toMatchObject({
      environment_id: ENV.id,
      manifest: 'api/worker-compose-next.yaml',
    })
    expect(of(h, 'iii-cloud::check')).toEqual([{ run: true, manifest: 'api/worker-compose-next.yaml' }])
  })

  it('a value iii cloud refused keeps the step: no check until it is saved or skipped', async () => {
    const refused = {
      environment_id: TEST_ENV.id,
      results: [{ name: 'APP_BASE_URL', ok: false, error: { code: 'NO_URL', message: 'no url' } }],
    }
    const h = await board('setup-values', {}, { 'iii-cloud::values::save': refused })
    await click(btn('Save 12 values and check'))
    await flush()
    expect(of(h, 'iii-cloud::check')).toEqual([])
    expect(text()).toContain('test has no address yet.')
    await click(btn('Check without saving'))
    expect(of(h, 'iii-cloud::check')).toEqual([{ run: true }])
  })

  it('all set: says so, folds the list under Show all, and Check checks', async () => {
    const h = await board('setup-values', {}, { 'iii-cloud::values': allSet })
    expect(text()).toContain('All 22 values api/worker-compose-prod.yaml reads are set in test.')
    expect(el.querySelectorAll('.iii-cloud-val-row')).toHaveLength(0)
    await click(btn('Show all 22'))
    expect(el.querySelectorAll('.iii-cloud-val-row')).toHaveLength(22)
    expect(btn('Check without saving')).toBeNull()
    await click(btn('Check'))
    expect(of(h, 'iii-cloud::values::save')).toEqual([])
    expect(of(h, 'iii-cloud::check')).toEqual([{ run: true }])
  })
})

describe('5 review and deploy (Onb6)', () => {
  it('a direct environment: what will happen, then Deploy calls deploy {check_id} and nothing else', async () => {
    const h = await board('setup-ship')
    expect(text()).toContain('Review and deploy to dev')
    expect(text()).toContain(
      'Commit 3c4d2e0 passed the check on iii cloud and here. Nothing goes live before you confirm.',
    )
    const [code, file, settings, creates, secrets, address] = facts()
    expect(code).toContain('2 commits by your agent on top of develop')
    expect(code).toContain('Goes to develop, the branch dev follows, when you confirm')
    expect(
      el.querySelector(`a[href="https://github.com/iii-hq/registry/compare/${ready.base}...${COMMIT}"]`),
    ).not.toBeNull()
    expect(file).toContain('9 workers in 3 groups.')
    expect(file).toContain('Your agent fixed: Lock missing, iii-sdk 0.24.4 is older than 0.26')
    expect(
      el.querySelector(`a[href="https://github.com/iii-hq/registry/blob/${COMMIT}/api/worker-compose-prod.yaml"]`),
    ).not.toBeNull()
    expect(settings).toContain('Changes 3 settings the console had')
    expect(settings).toContain('Web, healthcheck, release command now come from api/worker-compose-prod.yaml.')
    expect(settings).toContain('changes a setting')
    expect(creates).toBe('CreatesA shared Postgres for DATABASE_URL')
    expect(secrets).toBe('Secrets3 set · 1 optional not set')
    expect(address).toBe('Addresshttps://registry-dev.iii.cloud')
    expect(text()).toContain("Only you can confirm this; your agent can't.")
    expect(of(h, 'iii-cloud::deploy')).toEqual([])
    await click(btn('Deploy 3c4d2e0 to dev'))
    expect(of(h, 'iii-cloud::deploy')).toEqual([{ check_id: 'chk-ready' }])
  })

  it('production: the agent opens the pull request; on the checked commit, Merge and deploy merges it', async () => {
    const h = await mount(prodShip(), ctx({ at: 'setup' }))
    expect(facts()[0]).toContain('Goes to main through a pull request; merging it starts the deploy')
    expect(btn('Merge and deploy')).toBeNull()
    await click(btn('Open the pull request'))
    expect(of(h, 'iii-cloud::handoff')[0]).toMatchObject({ kind: 'pull_request', check_id: 'chk-ready' })
    expect(h.compose).toHaveBeenCalled()
    expect(of(h, 'iii-cloud::deploy')).toEqual([])

    const h2 = await mount(prodShip({ pr }), ctx({ at: 'setup' }, 2))
    expect(el.querySelector(`a[href="${pr.url}"]`)).not.toBeNull()
    await click(btn('Merge and deploy'))
    expect(of(h2, 'iii-cloud::deploy')).toEqual([{ check_id: 'chk-ready', pr: 12 }])
  })

  it('a stale pull request is never merged: closed, or open on another commit', async () => {
    await mount(prodShip({ pr: { ...pr, state: 'CLOSED' } }), ctx({ at: 'setup' }))
    expect(btn('Merge and deploy')).toBeNull()
    expect(btn('Open the pull request')).not.toBeNull()
    expect(text()).not.toContain('is closed')
    await mount(prodShip({ pr: { ...pr, head_sha: 'ffffffffffff' } }), ctx({ at: 'setup' }, 2))
    expect(btn('Merge and deploy')).toBeNull()
    expect(btn('Open the pull request')).not.toBeNull()
  })

  it('a merge iii cloud holds for approval: Confirm deploy confirms that deployment', async () => {
    const held = { ...DEPLOY_BASE, environment_id: ENV.id, status: 'needs_action' as const }
    const h = await mount(prodShip({ deploys: [held] }), ctx({ at: 'setup' }))
    expect(text()).toContain('iii cloud holds deploy #41 until you approve what it creates.')
    await click(btn('Confirm deploy'))
    expect(of(h, 'iii-cloud::deploy')).toEqual([{ check_id: 'chk-ready', deployment_id: DEPLOY_BASE.id }])
  })

  it('the base moved: the error offers Check again, and Deploy waits for it', async () => {
    const err = Object.assign(new Error('moved'), { code: 'MAIN_MOVED' })
    const h = await board('setup-ship', {}, { 'iii-cloud::deploy': err })
    await click(btn('Deploy 3c4d2e0 to dev'))
    expect(text()).toContain('The base branch moved after the check. Check again before you deploy.')
    expect(btn('Deploy 3c4d2e0 to dev')?.hasAttribute('disabled')).toBe(true)
    await click(btn('Check again'))
    expect(of(h, 'iii-cloud::check')).toEqual([{ run: true }])
  })

  it('the agent on the pull request or a deploy fix shows its row here', async () => {
    await mount(prodShip({ agent: { ...agent('working'), kind: 'pull_request' } }), ctx({ at: 'setup' }))
    expect(el.querySelector('.iii-cloud-agent')).not.toBeNull()
    await mount(prodShip({ agent: { ...agent('working'), kind: 'ready' } }), ctx({ at: 'setup' }, 2))
    expect(el.querySelector('.iii-cloud-agent')).toBeNull()
  })
})

describe('5 deploying (Onb6)', () => {
  it('the stages with the live one, the release log read from iii cloud, the full log', async () => {
    const h = await board('setup-deploying')
    await flush()
    expect(text()).toContain('Deploying 3c4d2e0 to dev')
    expect(text()).toMatch(/Deploy #5 · started \d{1,2}:\d{2}[^·]* · 4 min so far/)
    expect(el.querySelector('.iii-cloud-setup-headrow [aria-live=polite]')?.textContent).toBe('Workers')
    const stages = [...el.querySelectorAll('.iii-cloud-setup-stages li')]
    expect(stages.map((s) => s.hasAttribute('data-live'))).toEqual([false, false, false, false, false, true, false])
    expect(of(h, 'iii-cloud::api')).toContainEqual({
      method: 'GET',
      path: apiPath('/logs', DEV.id),
      query: { deployment_id: 'dep-5', source: 'release' },
    })
    expect(el.querySelector('[data-terminal=release]')?.textContent).toBe(
      'applying migration 0006_collections.sql\napplying migration 0007_vector_search.sql\n7 migrations applied',
    )
    expect(el.querySelector(`a[href="${deploying.console_url}"]`)?.textContent).toContain('Open full log')
  })

  it("the deploy's own lines need no read", async () => {
    const s = boards['setup-deploying'].status
    const h = await mount({ ...s, deploys: [{ ...deploying, log_tail: ['[http] ready'] }] }, ctx({ at: 'setup' }))
    expect(el.querySelector('[data-terminal=deploy]')?.textContent).toBe('[http] ready')
    expect(of(h, 'iii-cloud::api')).toEqual([])
  })

  it('Cancel the deploy asks first, then cancels it in iii cloud with a fresh key', async () => {
    const b = boards['setup-deploying']
    const cancel = apiPath('/deployments/dep-5/cancel', DEV.id)
    const h = await mount(b.status, ctx(b.context), cloudAnswers({ ...b.cloud, [cancel]: { id: 'dep-5' } }))
    await click(btn('Cancel the deploy'))
    expect(of(h, 'iii-cloud::api::write')).toEqual([])
    expect(el.querySelector('[role=alertdialog]')?.textContent).toContain('Cancel deploy #5?')
    await click(btn('Cancel deploy #5'))
    const writes = of(h, 'iii-cloud::api::write')
    expect(writes).toHaveLength(1)
    expect(writes[0]).toMatchObject({ method: 'POST', path: cancel })
    expect(typeof writes[0]?.idempotency_key).toBe('string')
  })

  it('while the cancel is in flight the dialog stays open, its confirm disabled: one cancel', async () => {
    const b = boards['setup-deploying']
    const h = await mount(b.status, ctx(b.context), {
      ...cloudAnswers(b.cloud!),
      'iii-cloud::api::write': () => new Promise(() => undefined),
    })
    await click(btn('Cancel the deploy'))
    await click(btn('Cancel deploy #5'))
    expect(el.querySelector('[role=alertdialog]')?.getAttribute('aria-busy')).toBe('true')
    expect(btn('Cancel deploy #5')?.hasAttribute('disabled')).toBe(true)
    await click(btn('Cancel deploy #5'))
    expect(of(h, 'iii-cloud::api::write')).toHaveLength(1)
  })

  it("a cancel iii cloud refuses says why, in iii cloud's words", async () => {
    const b = boards['setup-deploying']
    await mount(b.status, ctx(b.context), {
      ...cloudAnswers(b.cloud!),
      'iii-cloud::api::write': () => ({
        status: 409,
        body: { code: 'TOO_LATE', message: "The deploy is past Routes & TLS: it can't be canceled any more." },
      }),
    })
    await click(btn('Cancel the deploy'))
    await click(btn('Cancel deploy #5'))
    expect(el.querySelector('[role=status][data-variant=alert]')?.textContent).toContain(
      "The deploy is past Routes & TLS: it can't be canceled any more.",
    )
  })
})

describe('6 live (Onb7)', () => {
  it('the address, health from iii cloud, the commit shipped, how long it took; then the project', async () => {
    await board('setup-live')
    await flush()
    expect(text()).toContain('registry is live on dev')
    expect(text()).toContain(
      'Deploy #5 is serving commit 3c4d2e0. It took 1 h 20 min from the first check, with 2 failed deploys along the way.',
    )
    expect(el.querySelector('a[href="https://registry-dev.iii.cloud"]')?.textContent).toContain('Open')
    expect(btn('Copy')).not.toBeNull()
    expect([...el.querySelectorAll('.iii-cloud-setup-checks li')].map((li) => li.textContent)).toEqual([
      "iii cloud says it's healthy9 of 9 workers up",
      'Serving the commit you shippedversion 3c4d2e0',
    ])
    await click(btn('Go to registry'))
    expect(el.querySelector('aside[aria-label="Setup steps"]')).toBeNull()
    expect(text()).not.toContain('is live on dev')
  })

  it('a deploy of another commit is said plainly', async () => {
    const b = boards['setup-live']
    await mount(
      { ...b.status, check: { ...b.status.check!, commit: 'f'.repeat(40) } },
      ctx(b.context),
      cloudAnswers(b.cloud!),
    )
    expect(text()).toContain('Serving a commit other than the one you checked')
  })
})

describe('adding a project (add mode)', () => {
  /** billing, read beside registry: not imported yet into a project of its own, one cloud project of its repository. */
  const billingCp = {
    id: 'p-billing',
    name: 'billing',
    slug: 'billing',
    team: registry.team,
    environments: [{ id: 'e-b', name: 'production', production: true, branch: 'main', manifest: null }],
  }
  const candidate = {
    ...project(false),
    path: '/home/you/workspaces/billing',
    repository: 'iii-hq/billing',
    cloud_project: { id: 'p-billing', name: 'billing', slug: 'billing' },
    cloud_projects: [billingCp],
    environments: billingCp.environments,
    environment: null,
  }
  const adding: Status = { ...base, candidate }
  const atAdd = (step: string) => ctx({ at: 'setup', step, adding: true })
  /** The page's own calls, without the status and cloud reads. */
  const own = (h: { calls: Call[] }) =>
    h.calls.filter((c) => c.id !== 'iii-cloud::status' && c.id !== 'iii-cloud::api').map((c) => c.id)

  it('reads the candidate, read again on arrival; the project in use stays as it is', async () => {
    const h = await mount(adding, atAdd('code'))
    expect(el.querySelector('[role=toolbar] nav')?.textContent).toBe('Projects / Add a project')
    expect(of(h, 'iii-cloud::project::resolve')).toEqual([{ path: '/home/you/workspaces/billing' }])
    expect(facts()[0]).toContain('iii-hq/billing')
    expect((el.querySelector('[data-directory-picker]') as HTMLInputElement).value).toBe('/home/you/workspaces/billing')
  })

  it('Cancel on Code and on Target goes back to Start, with no call', async () => {
    for (const [i, step] of ['code', 'target'].entries()) {
      const h = await mount(adding, ctx({ at: 'setup', step, adding: true }, i + 1))
      expect(text()).toContain(step === 'code' ? 'Which code goes to the cloud?' : 'Where should it run?')
      const before = own(h).length
      await click(btn('Cancel'))
      expect(el.querySelector('[role=toolbar] nav')?.textContent).toBe('Projects')
      expect(own(h).slice(before)).toEqual([])
      expect(of(h, 'iii-cloud::project::use')).toEqual([])
    }
  })

  it("Target lists the candidate's cloud projects; Continue uses the candidate's folder, then its values", async () => {
    const h = await mount(adding, atAdd('target'), cloudAnswers({}))
    expect(text()).toContain('Where should it run?')
    expect(radios('Project').map((r) => r.textContent)).toEqual([
      'billingBuilt from iii-hq/billing · 1 environmentthis repository',
      'New projectImports iii-hq/billing and creates its first environment',
    ])
    await click(btn('Continue'))
    expect(of(h, 'iii-cloud::project::use')).toEqual([
      { path: '/home/you/workspaces/billing', cloud_project_id: 'p-billing', environment_id: 'e-b', mode: 'worktree' },
    ])
    expect(sent(h).slice(-1)).toEqual(['iii-cloud::values'])
    expect(of(h, 'iii-cloud::check')).toEqual([])
    // The values of the project now in use: add mode is over.
    expect(el.querySelector('[role=toolbar] nav')?.textContent).toBe('Projects / registry / Set up')
  })

  it('the rail’s Your code with another folder enters add mode; the folder resolves beside the project in use', async () => {
    const h = await mount(base, ctx({ at: 'setup' }))
    await click(row('Your code'))
    expect(el.querySelector('[role=toolbar] nav')?.textContent).toBe('Projects / registry / Set up')
    await type(el.querySelector('[data-directory-picker]'), '/home/you/workspaces/billing')
    expect(el.querySelector('[role=toolbar] nav')?.textContent).toBe('Projects / Add a project')
    expect(of(h, 'iii-cloud::project::resolve').at(-1)).toEqual({ path: '/home/you/workspaces/billing' })
    expect(btn('Cancel')).toBeTruthy()
  })

  it('a folder that fails to resolve shows no earlier candidate: no panel, Continue off, no use', async () => {
    const refused = Object.assign(new Error('No GitHub remote in this folder.'), { code: 'NO_GITHUB_REMOTE' })
    let n = 0
    const h = await mount(adding, atAdd('code'), {
      // The arrival read of billing succeeds; the folder typed next fails.
      'iii-cloud::project::resolve': () => {
        if (n++ > 0) throw refused
        return {}
      },
    })
    expect(facts()[0]).toContain('iii-hq/billing')
    await type(el.querySelector('[data-directory-picker]'), '/home/you/workspaces/notes')
    await flush()
    expect(text()).toContain('No GitHub remote in this folder.')
    expect(el.querySelector('[aria-label="What the worker found"]')).toBeNull()
    expect(text()).not.toContain('iii-hq/billing')
    expect((btn('Continue') as HTMLButtonElement).disabled).toBe(true)
    expect(of(h, 'iii-cloud::project::use')).toEqual([])
  })

  it('the import hand-off of a candidate sends no check of the project in use', async () => {
    const unseen = { ...candidate, github_access: false, cloud_project: null, cloud_projects: [], environments: [] }
    const h = fakeHost({ ...base, candidate: unseen }, { 'iii-cloud::handoff': { id: 'h', marker: '[m]' } })
    await act(async () => root.render(<CloudPage host={h.host} panelSide="right" tabId="t" {...atAdd('target')} />))
    await flush()
    await flush()
    await click(btn('Ask the agent to import it'))
    expect(of(h, 'iii-cloud::handoff')).toHaveLength(1)
    expect(of(h, 'iii-cloud::handoff')[0]).not.toHaveProperty('check_id')
    expect(of(h, 'iii-cloud::handoff')[0]).toMatchObject({ kind: 'import', candidate: true })
    expect(h.compose.mock.calls[0][0].text).toContain('Import iii-hq/billing')
  })
})
