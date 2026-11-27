// First time: sign in once, then one confirmation with the four rows (folder, project, environment, compose
// file), each changeable in place. What happens after the button is the Deploy place's.
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { CloudPage } from '../CloudPage'
import { boards, cardRecords } from '../fixtures'
import { base, project, TEAM } from '../fixtures/base'
import { chosen, DEV, FILE, notImported, registry } from '../fixtures/setup'
import { startStatus } from '../fixtures/start'
import { assistedHow } from '../model'
import type { Status } from '../types'
import { type Call, fakeHost } from './host'
import { board, btn, click, cloudAnswers, ctx, el, flush, lastCall, mount, root, text, type, usePage } from './util'

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
const path = () => el.querySelector('[role=toolbar] nav')?.textContent
/** The four rows: label to its value (first line) and note. */
const detail = (label: string) => {
  const r = [...el.querySelectorAll('.iii-cloud-detail')].find(
    (x) => x.querySelector('.iii-cloud-detail-label')?.textContent === label,
  )
  return r
    ? {
        value: r.querySelector('.iii-cloud-detail-value')?.firstChild?.textContent,
        note: r.querySelector('.iii-cloud-detail-note')?.textContent,
        change: r.querySelector('button') as HTMLButtonElement,
      }
    : null
}
const changeOf = (label: string) => detail(label)?.change
const opened = () => [...el.querySelectorAll('.iii-cloud-detail-open .iii-cloud-setup-label')].map((x) => x.textContent)
/** A status that takes the project up once `project::use` was called, as the worker does. */
const takesUp = (s: Status, env: { id: string; name: string; production: boolean; branch: string }) => {
  let used = false
  return {
    status: (() =>
      used
        ? {
            ...s,
            project: {
              ...s.project!,
              workspace: { mode: 'worktree' as const, path: '~/workspaces/registry-iii-cloud', branch: 'b' },
              environment: { ...env, manifest: FILE, url: null },
              environments: [env],
            },
          }
        : s) as unknown as Status,
    use: () => {
      used = true
      return {}
    },
  }
}
const primary = () => el.querySelector('button[data-variant=primary]') as HTMLButtonElement | null

describe('sign in (First1)', () => {
  it('waiting: the code, the wait, the page again, one note, the API key under a disclosure; no list of steps, no rail', async () => {
    await board('setup-signin')
    expect(el.querySelector('h2#iii-cloud-signin')?.textContent).toBe('Sign in to iii cloud')
    expect(text()).toContain('Check that it shows this code, then approve.')
    expect(text()).toContain('WXYZ-1234')
    expect(text()).toMatch(/Waiting for your approval · the code expires in 9 min/)
    const link = el.querySelector('a[href="https://cloud.iii.dev/device"]')
    expect(link?.getAttribute('target')).toBe('_blank')
    expect(link?.textContent).toBe('Open the page again')
    expect(el.innerHTML).not.toMatch(/href="[^"]*WXYZ/)
    expect(text()).toContain('The session stays on this machine. Your agent can use it but never reads it.')
    expect(text()).toContain('After you approve, this page goes on by itself.')
    expect(text()).not.toMatch(/You step in|Where you step in/)
    expect(el.querySelector('aside')).toBeNull()
    // Nothing to go back to on the first run.
    expect(btn('Cancel')).toBeNull()
    expect(path()).toBe('Put this project on iii cloud')
  })

  it('with projects on this machine, Cancel goes back to Projects', async () => {
    await mount({ ...startStatus, connection: boards['setup-signin'].status.connection }, ctx({ at: 'setup' }))
    expect(text()).toContain('WXYZ-1234')
    await click(btn('Cancel'))
    expect(path()).toBe('Projects')
  })

  it('signed out: the browser button calls connect; the API key waits behind a disclosure', async () => {
    const h = await mount({ ...boards['setup-signin'].status, connection: { ...cardRecords.signedOut } })
    expect(text()).not.toContain('Waiting for your approval')
    await click(btn('Sign in with the browser'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::connect', payload: { method: 'browser' } })
    expect(el.querySelector('#iii-cloud-secret-name')).toBeNull()
    await click(btn("Can't use a browser here? Use an API key"))
    expect((el.querySelector('#iii-cloud-secret-name') as HTMLInputElement).value).toBe('IIICLOUD_API_KEY')
    expect(text()).toContain("It can't create projects or environments, so adding a project needs the browser")
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
    expect(btn('Sign in with the browser')).toBeNull()
    expect(btn('Try again')?.getAttribute('data-variant')).toBe('secondary')
    expect(document.activeElement?.id).toBe('iii-cloud-secret-name')
  })

  it('SIGN_IN_REQUIRED brings the page back to the sign-in, which goes on by itself afterwards', async () => {
    const p = { ...base.check!.problems[0], id: 'sir', code: 'SIGN_IN_REQUIRED', owner: 'person' as const }
    const h = await mount({ ...base, check: { ...base.check!, problems: [p] } }, ctx({ at: 'setup' }, 2))
    expect(text()).toContain('Your sign-in expired')
    await click(btn('Sign in again'))
    expect(of(h, 'iii-cloud::connect')).toEqual([{ method: 'browser' }])
  })

  it('signed in, the sign-in is never asked again: the confirmation shows at once', async () => {
    await board('setup-existing', ctx({ at: 'setup', step: 'signin' }))
    expect(text()).toContain('Put registry on iii cloud')
    expect(text()).not.toContain('Sign in to iii cloud')
  })
})

describe('confirm (First2): four rows, each filled with what the page found', () => {
  it('a repository iii cloud has not imported: the folder, a new project, a new environment, the recommended compose file', async () => {
    await board('setup-new')
    expect(el.querySelector('.iii-cloud-setup-title')?.textContent).toBe('Put registry on iii cloud')
    expect(text()).toContain(
      "The page read your folder and filled this in. Change anything that's wrong, then continue.",
    )
    expect([...el.querySelectorAll('.iii-cloud-detail-label')].map((l) => l.textContent)).toEqual([
      'Folder',
      'Project',
      'Environment',
      'Compose file',
    ])
    expect(detail('Folder')).toMatchObject({
      value: '~/workspaces/registry',
      note: 'iii-hq/registry on GitHub · you are on main, up to date',
    })
    expect(detail('Project')).toMatchObject({
      value: 'New project: registry',
      note: 'No project in team iii is built from this repository yet',
    })
    expect(detail('Environment')).toMatchObject({
      value: 'New environment: test',
      note: `Assisted deploy. ${assistedHow('test')}`,
    })
    expect(detail('Compose file')).toMatchObject({ value: FILE, note: 'It has a cloud section · 9 workers' })
    for (const label of ['Folder', 'Project', 'Environment', 'Compose file']) {
      expect(changeOf(label)?.textContent).toBe('Change')
    }
    expect(text()).toContain(
      'Your agent works in a clean copy of what is committed, so your folder stays as it is. The 4 uncommitted changes in your folder stay there: commit them to include them.',
    )
    expect(el.querySelector('aside')).toBeNull()
  })

  it('the repository has a project: it, the environment that is not production, the environment in use is not asked', async () => {
    await board('setup-existing')
    expect(detail('Project')).toMatchObject({
      value: 'registry',
      note: 'It is built from this repository · 2 environments',
    })
    expect(detail('Environment')?.value).toBe('dev')
    expect(detail('Environment')?.note).toBe(`Assisted deploy. ${assistedHow('dev')}`)
    expect(detail('Compose file')?.value).toBe(FILE)
  })

  it('the button says what it does: "Create <project> and check it" or "Continue to the check", and what is created', async () => {
    await board('setup-new')
    expect(primary()?.textContent).toBe('Create registry and check it')
    expect(text()).toContain(
      'This creates the project and its test environment in iii cloud and starts the check. Nothing is deployed until you confirm a deploy.',
    )
    // Nothing to go back to on the first run.
    expect(btn('Cancel')).toBeNull()
  })

  it('"Continue to the check" when nothing is created', async () => {
    await board('setup-existing')
    expect(primary()?.textContent).toBe('Continue to the check')
    expect(text()).toContain('This starts the check of dev. Nothing is deployed until you confirm a deploy.')
  })

  it('the old steps are not reachable: their contexts land on the confirmation or the Deploy place', async () => {
    await mount({ ...base, project: chosen, check: null }, ctx({ at: 'setup', step: 'code' }))
    expect(text()).toContain('Put registry on iii cloud')
    expect(el.querySelector('aside[aria-label="Setup steps"]')).toBeNull()
    await mount(base, ctx({ at: 'setup', step: 'values' }, 2))
    expect(path()).toBe('Projects / registry · production / Deploy')
  })
})

describe('Change: each row opens in place, one at a time', () => {
  it('opening a row closes the other; Done closes it; nothing is called', async () => {
    const h = await board('setup-existing')
    await click(changeOf('Project') ?? null)
    expect(opened()).toEqual(['Project'])
    expect(detail('Project')).toBeNull()
    expect(detail('Folder')).not.toBeNull()
    await click(changeOf('Environment') ?? null)
    expect(opened()).toEqual(['Environment'])
    expect(detail('Project')).not.toBeNull()
    expect(text()).toContain('Picking one changes nothing in iii cloud.')
    await click(btn('Done'))
    expect(opened()).toEqual([])
    expect(sent(h)).toEqual([])
  })

  it('Folder: the picker, what was found, and "Let your agent edit this folder" closed by default', async () => {
    await board('setup-new')
    await click(changeOf('Folder') ?? null)
    expect((el.querySelector('[data-directory-picker]') as HTMLInputElement).value).toBe(
      '/home/you/workspaces/registry',
    )
    expect(text()).toContain('iii-hq/registry on GitHub')
    expect(text()).toContain('4 uncommitted changes; they stay in your folder')
    expect(text()).toContain('Another folder reads again')
    expect(el.querySelector('[role=alertdialog]')).toBeNull()
    expect(btn('Let your agent edit this folder instead of a copy')).toBeTruthy()
  })

  it('Folder: another folder reads again, with project::resolve, and the page stays here', async () => {
    const h = await board('setup-new')
    await click(changeOf('Folder') ?? null)
    await type(el.querySelector('[data-directory-picker]'), '/home/you/workspaces/other')
    expect(lastCall(h.calls)).toEqual({
      id: 'iii-cloud::project::resolve',
      payload: { path: '/home/you/workspaces/other' },
    })
    expect(text()).toContain('Put registry on iii cloud')
  })

  it('Folder: letting the agent edit the folder asks first; then the check uses it with no copy', async () => {
    const h = await board('setup-existing')
    await click(changeOf('Folder') ?? null)
    await click(btn('Let your agent edit this folder instead of a copy'))
    expect(el.querySelector('[role=alertdialog]')?.textContent).toContain('Let your agent edit your own files?')
    await click(btn('Edit my own files'))
    expect(text()).toContain('Your agent will change the files in this folder directly, with no separate copy.')
    await click(btn('Done'))
    expect(text()).toContain('Your agent changes the files in this folder directly, with no separate copy.')
    await click(primary())
    expect(of(h, 'iii-cloud::project::use')[0]).toMatchObject({ mode: 'checkout' })
    expect(of(h, 'iii-cloud::project::use')[0]).not.toHaveProperty('bring_changes')
  })

  it('Project: the project of this repository, "New project", the other repositories by name; the choice changes the row and the button', async () => {
    await board('setup-existing')
    await click(changeOf('Project') ?? null)
    expect(radios('Project').map((r) => r.textContent)).toEqual([
      'registryBuilt from iii-hq/registry · 2 environments',
      'New projectImports iii-hq/registry and creates its first environment',
    ])
    expect(checked('Project')).toMatch(/^registry/)
    expect(text()).toContain(
      '3 other projects in team iii are built from other repositories: hello-iii, docs-site, learn-iii. Open their folder to work on them.',
    )
    await click(radio('Project', 'New project') ?? null)
    await click(btn('Done'))
    expect(detail('Project')?.value).toBe('New project: registry')
    expect(detail('Environment')?.value).toBe('New environment: test')
    expect(primary()?.textContent).toBe('Create registry and check it')
  })

  it('Environment: the environments with what iii cloud has live, a new one with its name and branch', async () => {
    await board('setup-existing')
    await click(changeOf('Environment') ?? null)
    expect(text()).toContain(
      'Each environment is one live copy of the app, with its own address, database and secrets.',
    )
    expect(radios('Environment').map((r) => r.textContent)).toEqual([
      'productionDeploys when a pull request into main is merged.',
      'devAssisted deployLive · 9c04e7f',
      'New environmentYou pick a name. Its secrets and settings start as a copy, and it gets an assisted deploy.',
    ])
    // The one sentence of the assisted deploy is said once, under the rows.
    expect(text()).toContain(`Assisted deploy: ${assistedHow('the environment')}`)
    expect(checked('Environment')).toMatch(/^dev/)
    await click(radio('Environment', 'production') ?? null)
    await click(btn('Done'))
    expect(detail('Environment')).toMatchObject({
      value: 'production',
      note: 'Deploys when a pull request into main is merged.',
    })
    await click(changeOf('Environment') ?? null)
    await click(radio('Environment', 'New environment') ?? null)
    await type(el.querySelector('#iii-cloud-ne-name'), 'staging')
    expect((el.querySelector('#iii-cloud-ne-branch') as HTMLInputElement).value).toBe('main')
    await click(btn('Done'))
    expect(detail('Environment')?.value).toBe('New environment: staging')
    expect(primary()?.textContent).toBe('Create staging and check it')
  })

  it('New environment: the branch hint says whether it gets its own branch (default, shared, own)', async () => {
    await board('setup-existing')
    await click(changeOf('Environment') ?? null)
    await click(radio('Environment', 'New environment') ?? null)
    await type(el.querySelector('#iii-cloud-ne-name'), 'staging')
    const hint = () => el.querySelector('#iii-cloud-ne-branch-hint')?.textContent
    const branch = el.querySelector('#iii-cloud-ne-branch')
    expect(el.querySelector('label[for=iii-cloud-ne-branch]')?.textContent).toBe('Branch')
    // The default branch: never pushed.
    expect(hint()).toBe('Its first assisted deploy gives it its own branch, iii-cloud/staging. main is never pushed.')
    // A branch another environment follows (dev's).
    await type(branch, 'develop')
    expect(hint()).toBe('Its first assisted deploy gives it its own branch, iii-cloud/staging. develop is never pushed.')
    // A branch nobody follows.
    await type(branch, 'feature')
    expect(hint()).toBe(
      'An assisted deploy sends the checked commit to feature. A push to feature from anywhere else deploys without the check.',
    )
  })

  it('Environment of a new project: its name and whether it is production', async () => {
    await board('setup-new')
    await click(changeOf('Environment') ?? null)
    await type(el.querySelector('#iii-cloud-np-env'), 'prod')
    await click(el.querySelector('input[type=checkbox]'))
    await click(btn('Done'))
    expect(detail('Environment')).toMatchObject({
      value: 'New environment: prod',
      note: 'Deploys when a pull request into main is merged.',
    })
  })

  it('Compose file: the files, the recommended one marked, one iii cloud cannot take off; the pick changes the row', async () => {
    await board('setup-existing')
    await click(changeOf('Compose file') ?? null)
    expect(radios('Compose file the cloud runs').map((r) => r.textContent)).toEqual([
      `${FILE}Recommended · It has a cloud section · 11 workers`,
      'api/worker-compose.staging.yaml11 workers',
      'api/workers-compose.yamlnot a worker-compose*.yaml',
    ])
    expect(radio('Compose file the cloud runs', 'api/workers-compose.yaml')?.getAttribute('aria-disabled')).toBe('true')
    await click(radio('Compose file the cloud runs', 'api/worker-compose.staging.yaml') ?? null)
    await click(btn('Done'))
    expect(detail('Compose file')?.value).toBe('api/worker-compose.staging.yaml')
  })
})

describe('the button: the calls in order, then the Deploy place', () => {
  it('an existing project and environment: use, then check; the file picked goes to the check; then Deploy', async () => {
    const b = boards['setup-existing']
    const up = takesUp(b.status, DEV)
    const h = await mount(up.status, ctx(b.context), { ...cloudAnswers(b.cloud!), 'iii-cloud::project::use': up.use })
    await click(changeOf('Compose file') ?? null)
    await click(radio('Compose file the cloud runs', 'api/worker-compose.staging.yaml') ?? null)
    await click(btn('Done'))
    await click(primary())
    expect(sent(h).slice(0, 2)).toEqual(['iii-cloud::project::use', 'iii-cloud::check'])
    expect(of(h, 'iii-cloud::project::use')).toEqual([
      {
        path: '/home/you/workspaces/registry',
        cloud_project_id: 'p1',
        environment_id: DEV.id,
        mode: 'worktree',
      },
    ])
    expect(of(h, 'iii-cloud::check')).toEqual([{ run: true, manifest: 'api/worker-compose.staging.yaml' }])
    expect(path()).toBe('Projects / registry · dev / Deploy')
  })

  it('the recommended file is not sent: the check plans the environment’s own', async () => {
    const h = await board('setup-existing')
    await click(primary())
    expect(of(h, 'iii-cloud::check')).toEqual([{ run: true }])
  })

  it('a new project: create, use the made ids, check; its first environment is the one checked', async () => {
    const made = {
      project_id: 'p-new',
      environment_id: 'e-new',
      cloud_projects: [
        {
          id: 'p-new',
          name: 'registry',
          slug: 'registry',
          team: { id: TEAM.id, name: TEAM.name },
          environments: [{ id: 'e-new', name: 'test', production: false, branch: 'develop', manifest: FILE }],
        },
      ],
    }
    const h = await board('setup-new', {}, { 'iii-cloud::project::create': made })
    await click(primary())
    expect(sent(h)).toEqual(['iii-cloud::project::create', 'iii-cloud::project::use', 'iii-cloud::check'])
    expect(of(h, 'iii-cloud::project::create')).toEqual([
      {
        path: '/home/you/workspaces/registry',
        team_id: TEAM.id,
        manifest: FILE,
        environment_name: 'test',
        production: false,
      },
    ])
    expect(of(h, 'iii-cloud::project::use')).toEqual([
      {
        path: '/home/you/workspaces/registry',
        cloud_project_id: 'p-new',
        environment_id: 'e-new',
        mode: 'worktree',
      },
    ])
    expect(of(h, 'iii-cloud::check')).toEqual([{ run: true }])
  })

  it('a new environment in the project: create_environment copies a trial environment, then use, then check', async () => {
    const made = {
      project_id: 'p1',
      environment_id: 'e-stg',
      cloud_projects: [
        {
          ...registry,
          environments: [
            ...registry.environments!,
            { id: 'e-stg', name: 'staging', production: false, branch: 'main', manifest: FILE },
          ],
        },
      ],
    }
    const b = boards['setup-existing']
    const up = takesUp(b.status, { id: 'e-stg', name: 'staging', production: false, branch: 'main' })
    const h = await mount(up.status, ctx(b.context), {
      ...cloudAnswers(b.cloud!),
      'iii-cloud::project::create_environment': made,
      'iii-cloud::project::use': up.use,
    })
    await click(changeOf('Environment') ?? null)
    await click(radio('Environment', 'New environment') ?? null)
    await type(el.querySelector('#iii-cloud-ne-name'), 'staging')
    await click(btn('Done'))
    await click(primary())
    expect(sent(h).slice(0, 3)).toEqual([
      'iii-cloud::project::create_environment',
      'iii-cloud::project::use',
      'iii-cloud::check',
    ])
    expect(of(h, 'iii-cloud::project::create_environment')).toEqual([
      { cloud_project_id: 'p1', name: 'staging', branch: 'main', manifest: FILE, copy_from: DEV.id },
    ])
    expect(of(h, 'iii-cloud::project::use')[0]).toMatchObject({ cloud_project_id: 'p1', environment_id: 'e-stg' })
    expect(path()).toBe('Projects / registry · staging / Deploy')
  })

  it('a create iii cloud refuses stops there: its own sentence, no use, no check, and the button stays', async () => {
    const err = Object.assign(new Error('Only an Owner can create projects. (Your role is Developer.)'), {
      code: 'TEAM_FORBIDDEN',
    })
    const h = await board('setup-new', {}, { 'iii-cloud::project::create': err })
    await click(primary())
    expect(sent(h)).toEqual(['iii-cloud::project::create'])
    expect(text()).toContain('iii cloud says: Your role is Developer.')
    expect(primary()?.disabled).toBe(false)
    expect(path()).toBe('Put this project on iii cloud')
  })

  it('PROJECT_DELETED on use: a panel with "Choose another", which picks another project', async () => {
    const other = { ...registry, id: 'p2', name: 'registry-eu', environments: [{ ...DEV, id: 'e-eu' }] }
    const s: Status = { ...base, project: { ...chosen, cloud_projects: [registry, other] }, check: null }
    const gone = Object.assign(new Error('registry no longer exists in iii cloud.'), { code: 'PROJECT_DELETED' })
    await mount(s, ctx({ at: 'setup', step: 'confirm' }), { 'iii-cloud::project::use': gone })
    await click(primary())
    expect(text()).toContain('registry no longer exists in iii cloud.')
    await click(btn('Choose another'))
    expect(detail('Project')?.value).toBe('registry-eu')
    expect(detail('Environment')?.value).toBe('dev')
  })

  it('a folder with nothing uncommitted: the copy sentence has no second part', async () => {
    const clean = { ...notImported, checkout: { ...notImported.checkout, changed_files: [] } }
    await mount({ ...base, project: clean, check: null }, ctx({ at: 'setup', step: 'confirm' }))
    expect(text()).toContain('Your agent works in a clean copy of what is committed, so your folder stays as it is.')
    expect(text()).not.toContain('uncommitted change')
  })

  it('iii cloud cannot see the repository on GitHub: the button waits and says why; the agent imports it', async () => {
    const unseen = { ...notImported, github_access: false }
    const h = fakeHost({ ...base, project: unseen, check: null }, { 'iii-cloud::handoff': { id: 'h', marker: '[m]' } })
    await act(async () =>
      root.render(<CloudPage host={h.host} panelSide="right" tabId="t" {...ctx({ at: 'setup', step: 'confirm' })} />),
    )
    await flush()
    expect(primary()?.disabled).toBe(true)
    expect(text()).toContain("iii cloud can't see iii-hq/registry on GitHub yet.")
    await click(btn('Ask the agent to import it'))
    expect(of(h, 'iii-cloud::handoff')[0]).toMatchObject({ kind: 'import' })
    expect(h.compose.mock.calls[0][0].text).toContain('Import iii-hq/registry')
  })

  it('a new environment with no name yet: the button waits and says why', async () => {
    await board('setup-existing')
    await click(changeOf('Environment') ?? null)
    await click(radio('Environment', 'New environment') ?? null)
    await click(btn('Done'))
    expect(primary()?.disabled).toBe(true)
    expect(text()).toContain('Name the new environment first.')
  })
})

describe('no folder known yet, and adding a project', () => {
  it('no project yet: the chat folder resolves on arrival; the picker shows until it does', async () => {
    const h = await mount(
      { ...boards['setup-new'].status, project: null },
      { ...ctx({ at: 'setup', step: 'confirm' }), workingDir: '/home/you/workspaces/registry' },
    )
    expect(of(h, 'iii-cloud::project::resolve')).toEqual([{ path: '/home/you/workspaces/registry' }])
    expect(primary()).toBeNull()
  })

  it('no project and no folder: the picker is open, and the page asks nothing else', async () => {
    await mount({ ...base, project: null, check: null }, ctx({ at: 'setup' }))
    expect(text()).toContain('Pick the folder of your project. The page reads it and fills in the rest.')
    expect(el.querySelector('[data-directory-picker]')).not.toBeNull()
    expect(primary()).toBeNull()
  })

  it('the first step: the Folder editor is in a card, the picker says "Choose folder", no Done and no note about a button below', async () => {
    await mount({ ...base, project: null, check: null }, ctx({ at: 'setup' }))
    expect(el.querySelector('.iii-cloud-detailcard [data-directory-picker]')?.getAttribute('placeholder')).toBe(
      'Choose folder',
    )
    expect(btn('Done')).toBeNull()
    expect(text()).not.toContain('Only the button below does')
    expect(el.querySelector('.iii-cloud-setup-title')?.textContent).toBe("Choose your project's folder")
    // Who is signed in is in the bar, as on the board.
    expect(el.querySelector('[role=toolbar]')?.textContent).toMatch(/@.+ · team .+\(.+\)/)
  })

  const candidate = {
    ...project(false),
    path: '/home/you/workspaces/billing',
    repository: 'iii-hq/billing',
    cloud_project: { id: 'p-billing', name: 'billing', slug: 'billing' },
    cloud_projects: [
      {
        id: 'p-billing',
        name: 'billing',
        slug: 'billing',
        team: registry.team,
        environments: [{ id: 'e-b', name: 'production', production: true, branch: 'main', manifest: null }],
      },
    ],
    environments: [{ id: 'e-b', name: 'production', production: true, branch: 'main' }],
    environment: null,
  }
  const stale: Status = { ...base, candidate }
  const add = ctx({ at: 'setup', adding: true })
  const projectsOf = (t: string) => `/teams/${t}/projects`
  const listed = [
    { id: 'p-billing', name: 'billing', repository: { full_name: 'iii-hq/billing' } },
    { id: 'p-docs', name: 'docs-site', repository: { full_name: 'iii-hq/docs-site' } },
  ]
  const cloud = cloudAnswers({ [projectsOf(TEAM.id)]: { projects: listed } })
  const pick = async (label: string) => {
    const r = [...el.querySelectorAll('.iii-cloud-add-choices [role=row]')].find((x) => x.textContent?.startsWith(label))
    await click(r ?? null)
  }
  const folder = async (dir: string) => {
    await type(el.querySelector('[data-directory-picker]'), dir)
    await flush()
  }

  it('starts clean: both ways in, nothing read, even with the chat folder and an earlier candidate', async () => {
    const h = await mount(stale, { ...add, workingDir: '/home/you/workspaces/billing' }, cloud)
    expect(path()).toBe('Projects / Add a project')
    expect(el.querySelector('.iii-cloud-setup-title')?.textContent).toBe('Add a project')
    expect([...el.querySelectorAll('.iii-cloud-add-choices [role=row]')].map((r) => r.textContent)).toEqual([
      'Use a project already in iii cloudPick the project, then its folder on this machine. Add an environment or deploy from here.',
      'Create a new project in iii cloudPick a folder on this machine. The page reads its repository and creates the project with its first environment.',
    ])
    expect(text()).toContain(
      'Either way you confirm the folder, the environment and the compose file, then the check starts. Nothing changes in iii cloud before that.',
    )
    expect(of(h, 'iii-cloud::project::resolve')).toEqual([])
    expect(detail('Folder')).toBeNull()
    await click(btn('Cancel'))
    expect(path()).toBe('Projects')
  })

  it('option 1 is off, with the reason, when the account has no project in iii cloud', async () => {
    await mount(stale, add, cloudAnswers({ [projectsOf(TEAM.id)]: { projects: [] } }))
    const first = el.querySelector('.iii-cloud-add-choices [role=row]')
    expect(first?.getAttribute('aria-disabled')).toBe('true')
    expect(first?.textContent).toContain('You have no project in iii cloud yet.')
  })

  it('option 1: the list, the folder of that repository, then the confirmation with the project fixed; both paths end in the same check', async () => {
    const h = await mount(stale, add, {
      ...cloud,
      'iii-cloud::project::resolve': () => candidate,
    })
    await pick('Use a project already in iii cloud')
    expect(el.querySelector('.iii-cloud-setup-title')?.textContent).toBe('Which project?')
    expect([...el.querySelectorAll('[aria-label="Projects in iii cloud"] [role=row]')].map((r) => r.textContent)).toEqual([
      'billingBuilt from iii-hq/billing',
      'docs-siteBuilt from iii-hq/docs-site',
    ])
    expect(el.querySelector('input[aria-label="Filter projects"]')).toBeNull()
    await click([...el.querySelectorAll('[aria-label="Projects in iii cloud"] [role=row]')][0])
    expect(el.querySelector('.iii-cloud-setup-title')?.textContent).toBe('Where is it on this machine?')
    expect(el.querySelector('[data-directory-picker]')?.getAttribute('placeholder')).toBe('Choose folder')
    expect(of(h, 'iii-cloud::project::resolve')).toEqual([])
    await folder('/home/you/workspaces/billing')
    expect(of(h, 'iii-cloud::project::resolve')).toEqual([
      { path: '/home/you/workspaces/billing', cloud_project_id: 'p-billing' },
    ])
    expect(detail('Folder')?.value).toBe('~/workspaces/billing')
    expect(detail('Project')?.value).toBe('billing')
    // Change on the Project row goes back to the list.
    await click(changeOf('Project') ?? null)
    expect(el.querySelector('.iii-cloud-setup-title')?.textContent).toBe('Which project?')
    await click([...el.querySelectorAll('[aria-label="Projects in iii cloud"] [role=row]')][0])
    await folder('/home/you/workspaces/billing')
    expect(of(h, 'iii-cloud::project::use')).toEqual([])
    await click(primary())
    expect(of(h, 'iii-cloud::project::use')).toEqual([
      { path: '/home/you/workspaces/billing', cloud_project_id: 'p-billing', environment_id: 'e-b', mode: 'worktree' },
    ])
    expect(of(h, 'iii-cloud::check')).toEqual([{ run: true }])
    expect(path()).toContain('Deploy')
  })

  it('a folder of another repository: the sentence in place, the picker stays', async () => {
    const mismatch = Object.assign(
      new Error('This folder is iii-hq/notes. billing is built from iii-hq/billing: pick a folder of that repository.'),
      { code: 'REPOSITORY_MISMATCH' },
    )
    await mount(stale, add, { ...cloud, 'iii-cloud::project::resolve': () => { throw mismatch } })
    await pick('Use a project already in iii cloud')
    await click([...el.querySelectorAll('[aria-label="Projects in iii cloud"] [role=row]')][0])
    await folder('/home/you/workspaces/notes')
    expect(text()).toContain(
      'This folder is iii-hq/notes. billing is built from iii-hq/billing: pick a folder of that repository.',
    )
    expect(el.querySelector('[data-directory-picker]')).not.toBeNull()
    expect(detail('Folder')).toBeNull()
    expect(primary()).toBeNull()
  })

  it('option 2 with a repository that already has a project: two actions, "Use" continues as option 1', async () => {
    const h = await mount(stale, add, { ...cloud, 'iii-cloud::project::resolve': () => candidate })
    await pick('Create a new project in iii cloud')
    await folder('/home/you/workspaces/billing')
    expect(text()).toContain('iii-hq/billing is already billing in iii cloud.')
    expect(btn('Create another project')).toBeTruthy()
    await click(btn('Use billing'))
    expect(detail('Project')?.value).toBe('billing')
    expect(of(h, 'iii-cloud::project::resolve')).toEqual([{ path: '/home/you/workspaces/billing' }])
  })

  it('option 2, "Create another project": the confirmation with a new project', async () => {
    await mount(stale, add, { ...cloud, 'iii-cloud::project::resolve': () => candidate })
    await pick('Create a new project in iii cloud')
    await folder('/home/you/workspaces/billing')
    await click(btn('Create another project'))
    expect(detail('Project')?.value).toBe('New project: billing')
  })

  it('option 2 with a repository iii cloud does not have: straight to the confirmation, ending in the same check', async () => {
    const fresh = { ...candidate, cloud_project: null, cloud_projects: [], environments: [] }
    const h = await mount({ ...base, candidate: fresh }, add, {
      ...cloud,
      'iii-cloud::project::resolve': () => fresh,
      'iii-cloud::project::create': () => ({ project_id: 'p-new', environment_id: 'e-new', cloud_projects: [] }),
    })
    await pick('Create a new project in iii cloud')
    await folder('/home/you/workspaces/billing')
    expect(detail('Project')?.value).toBe('New project: billing')
    expect(btn('Create another project')).toBeNull()
  })

  it("the agent's copy is refused in place, with the main checkout offered", async () => {
    const copy = Object.assign(
      new Error('This is the copy your agent works in. Pick your own folder: /home/you/workspaces/billing'),
      { code: 'AGENT_COPY', fix: '/home/you/workspaces/billing' },
    )
    const h = await mount(stale, add, {
      ...cloud,
      'iii-cloud::project::resolve': (p: { path: string }) => {
        if (p.path.endsWith('-iii-cloud')) throw copy
        return candidate
      },
    })
    await pick('Create a new project in iii cloud')
    await folder('/home/you/workspaces/billing-iii-cloud')
    expect(text()).toContain('This is the copy your agent works in. Pick your own folder: /home/you/workspaces/billing')
    expect(detail('Folder')).toBeNull()
    await click(btn('Use /home/you/workspaces/billing'))
    expect(of(h, 'iii-cloud::project::resolve').at(-1)).toEqual({ path: '/home/you/workspaces/billing' })
  })

  it('more than eight projects: a filter', async () => {
    const many = Array.from({ length: 9 }, (_, i) => ({ id: `p${i}`, name: `app-${i}`, repository: { full_name: `o/r${i}` } }))
    await mount(stale, add, cloudAnswers({ [projectsOf(TEAM.id)]: { projects: many } }))
    await pick('Use a project already in iii cloud')
    const input = el.querySelector('input[aria-label="Filter projects"]')
    expect(input).not.toBeNull()
    await type(input, 'app-3')
    expect(el.querySelectorAll('[aria-label="Projects in iii cloud"] [role=row]')).toHaveLength(1)
  })

  it('the old context with newProject lands on option 2\'s folder step', async () => {
    const h = await mount(stale, ctx({ at: 'setup', newProject: true }), cloud)
    expect(el.querySelector('[data-directory-picker]')).not.toBeNull()
    expect(of(h, 'iii-cloud::project::resolve')).toEqual([])
  })
})
