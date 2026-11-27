// The Deploy screen (DeployArea): one screen driven by state, no step rail. The check and Ready are CheckStage's
// (its own suite); here: opening with no check, Change, the deployment (stages, live, failed on the cloud, held), and
// the way out of each.
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { CloudPage } from '../CloudPage'
import { boards } from '../fixtures'
import { apiPath } from '../fixtures/base'
import { DEV, live6, running6, serving5 } from '../fixtures/deploy'
import { forgetHubMemory, hubOf } from '../hub'
import type { BranchRef, Status } from '../types'
import { type Call, fakeHost } from './host'
import { board, btn, click, cloudAnswers, ctx, el, flush, lastCall, mount, root, text, type, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

const worker = (calls: Call[]) =>
  calls.filter((c) => !['iii-cloud::status', 'iii-cloud::values', 'iii-cloud::api'].includes(c.id))
/** iii cloud's list of dev's deployments: #5 serves (the Change note and the Ready footer name it from this). */
const cloudList = (current: typeof serving5 | null = serving5) =>
  cloudAnswers({
    [apiPath('/deployments', DEV.id)]: {
      deployments: [serving5, live6].map((d) => ({
        id: d.id,
        number: d.number,
        environment_id: DEV.id,
        type: 'push',
        status: 'ready',
        current: d.id === current?.id,
        commit: { sha: d.commit.sha },
        triggered_by: { user_id: null, api_key_id: null },
        created_at: d.created_at,
        finished_at: null,
      })),
    },
  })
const title = () => el.querySelector('h2.iii-cloud-h')?.textContent
const path = () => el.querySelector('[role=toolbar] nav')?.textContent
const radio = (label: string) =>
  [...el.querySelectorAll('[role=radio]')].find((r) => r.textContent?.startsWith(label)) as HTMLElement | undefined
const region = (name: string) => el.querySelector(`[role=region][aria-label="${name}"]`)
const route = { at: 'deploy', env: DEV.id } as const

describe('one screen, no step rail', () => {
  it.each([
    'deploy-what',
    'deploy-review',
    'deploy-follow-running',
    'deploy-follow-live',
    'deploy-follow-failed',
  ] as const)('%s: a path bar and the screen; no rail, no "What goes" step', async (id) => {
    await board(id)
    expect(path()).toBe('Projects / registry · dev / Deploy')
    expect(el.querySelector('aside')).toBeNull()
    expect(text()).not.toMatch(/Check before deploy|What goes to dev\?|Review and deploy|app file/i)
  })

  it('opening it with no check starts nothing: what goes, and Check', async () => {
    const h = await board('deploy-what')
    expect(title()).toBe('Check feat/search-rank for dev')
    expect(text()).toContain('feat/search-rank: 3 commits and your 2 uncommitted files, to dev')
    expect(worker(h.calls)).toEqual([])
    expect(btn('Deploy to dev')?.hasAttribute('disabled')).toBe(true)
    await click(btn('Check'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
  })

  it('Ready: Deploy, then the same screen follows the deployment', async () => {
    const h = await board('deploy-review', {}, { 'iii-cloud::deploy': { ...running6 } })
    expect(title()).toBe('Ready to deploy to dev')
    const rows = region('What this deploy does')?.textContent ?? ''
    expect(rows).toContain("6 commits dev doesn't have")
    expect(rows).toContain('3 of yours, 1 with your 2 uncommitted files, 2 fixes by your agent')
    expect(rows).toContain('11 workers in 3 groups')
    expect(el.querySelector('input[type=checkbox]')).toBeNull()
    await click(btn('Deploy 9c04e7f to dev'))
    expect(h.calls.find((c) => c.id === 'iii-cloud::deploy')?.payload).toEqual({ check_id: 'chk-dev' })
  })

  it('a deployment that is not this environment’s is not followed', async () => {
    await board('deploy-follow-running', ctx({ at: 'deploy', env: DEV.id, deploy: 'dep-elsewhere' }))
    expect(text()).toContain('Deployment not found')
    await click(btn('Back to dev'))
    expect(path()).toBe('Projects / registry · dev')
  })
})

describe('Change', () => {
  it('opens under the header with the branch, the uncommitted files and the compose file; Cancel closes it', async () => {
    const h = await board('deploy-what', {}, cloudList())
    expect(region('What goes to dev')).toBeNull()
    await click(btn('Change'))
    const panel = region('What goes to dev')
    expect(panel?.textContent).toContain('dev serves d738d2f · deployment #5')
    expect(panel?.textContent).toContain('The check starts from feat/search-rank.')
    expect(panel?.textContent).toContain('Include my 2 uncommitted files')
    // .env never goes: it says it stays.
    expect(panel?.textContent).toContain('Not brought: .env (secrets stay on this machine).')
    expect(el.querySelectorAll('[aria-label="Compose files in this folder"] [role=radio]')).toHaveLength(3)
    expect(radio('api/workers-compose.yaml')?.getAttribute('aria-disabled')).toBe('true')
    // The environment is not changed here: it says to open the other one, and that control is on screen.
    expect(panel?.textContent).toContain('to deploy somewhere else, open that environment from Projects')
    await click(btn('Cancel'))
    expect(region('What goes to dev')).toBeNull()
    expect(worker(h.calls)).toEqual([])
  })

  it('with the branches the worker lists, Code is a choice: the folder first, then the others', async () => {
    const s = boards['deploy-what'].status
    const p = s.project as NonNullable<Status['project']>
    const ref = (name: string, ahead: number, over: Partial<BranchRef> = {}): BranchRef => ({
      name,
      remote: name.startsWith('origin/'),
      sha: 'c'.repeat(40),
      committed_at: new Date().toISOString(),
      ahead,
      is_env_branch: false,
      ...over,
    })
    const branches = [ref('feat/search-rank', 3), ref('main', 2), ref('origin/fix-billing', 1)]
    const h = await mount({ ...s, project: { ...p, checkout: { ...p.checkout, branches } } }, ctx(route))
    await click(btn('Change'))
    expect(radio('feat/search-rank')?.getAttribute('aria-checked')).toBe('true')
    expect(radio('feat/search-rank')?.textContent).toContain("your folder · 3 commits dev doesn't have")
    expect(radio('main')?.textContent).toContain("2 commits dev doesn't have")
    await click(radio('main'))
    expect(region('What goes to dev')?.textContent).not.toContain('Include my 2 uncommitted files')
    await click(btn('Check this instead'))
    const use = worker(h.calls)[0]
    expect(use.payload).toMatchObject({ from: 'checkout', source: 'main', bring_changes: false })
  })

  it('Check this instead: project::use with the pick, then the check with the file; the panel closes', async () => {
    const h = await board('deploy-what')
    await click(btn('Change'))
    await click(radio('api/worker-compose.staging.yaml'))
    await click(el.querySelector('input[type=checkbox]'))
    await click(btn('Check this instead'))
    const calls = worker(h.calls)
    expect(calls.map((c) => c.id)).toEqual(['iii-cloud::project::use', 'iii-cloud::check'])
    expect(calls[0].payload).toMatchObject({
      environment_id: DEV.id,
      mode: 'worktree',
      from: 'checkout',
      bring_changes: false,
    })
    expect(calls[1].payload).toEqual({ run: true, push: true, manifest: 'api/worker-compose.staging.yaml' })
    expect(region('What goes to dev')).toBeNull()
  })

  it('restarts the check: the agent working in the copy is stopped first', async () => {
    const withAgent: Status = {
      ...boards['deploy-what'].status,
      agent: { ...boards['deploy-follow-running'].status.agent, handoff_id: 'ho-1', state: 'working' } as never,
    }
    const h = await mount(withAgent, ctx(route), { 'iii-cloud::handoff::stop': {} })
    await click(btn('Change'))
    await click(btn('Check this instead'))
    expect(worker(h.calls).map((c) => c.id)).toEqual([
      'iii-cloud::handoff::stop',
      'iii-cloud::project::use',
      'iii-cloud::check',
    ])
    expect(worker(h.calls)[0].payload).toEqual({ id: 'ho-1' })
  })

  it('Projects goes to the projects', async () => {
    await board('deploy-what')
    await click(btn('Change'))
    await click(btn('Projects'))
    expect(path()).toBe('Projects')
  })
})

describe('the deployment', () => {
  it('running: the stages with the release row, the live log from the cloud, and dev keeps serving #5', async () => {
    await board('deploy-follow-running')
    expect(title()).toBe('Deploying 9c04e7f to dev')
    expect(text()).toMatch(/Deployment #6 · started \d\d:\d\d · \d+ min so far/)
    const stages = [...(region('Stages')?.querySelectorAll('[role=row]') ?? [])].map((r) => r.textContent)
    expect(stages).toHaveLength(7)
    expect(stages.some((s) => s?.startsWith('Release command'))).toBe(true)
    expect(text()).toContain('search: loading trigram index (1.2M rows)')
    expect(text()).toContain('dev keeps serving #5 until #6 is live.')
    expect(btn('Back to dev')).toBeNull()
  })

  it('running from this machine: Cancel the deployment asks first, then posts the cancel to iii cloud', async () => {
    const h = await board('deploy-follow-running', {}, { 'iii-cloud::api::write': { status: 202, body: {} } })
    const writes = () => h.calls.filter((c) => c.id === 'iii-cloud::api::write')
    await click(btn('Cancel the deployment'))
    expect(writes()).toEqual([])
    expect(text()).toContain('Cancel deployment #6?')
    await click(btn('Keep deploying'))
    expect(writes()).toEqual([])
    await click(btn('Cancel the deployment'))
    await click(btn('Cancel deployment #6'))
    await flush()
    expect(writes()).toHaveLength(1)
    expect(writes()[0].payload).toMatchObject({
      method: 'POST',
      path: expect.stringContaining('/deployments/dep-6/cancel'),
    })
  })

  it('live: the address with Open and Copy, health, and Back to dev', async () => {
    await board('deploy-follow-live')
    expect(title()).toBe('dev is serving 9c04e7f')
    expect(text()).toMatch(/Deployment #6 · live at \d\d:\d\d/)
    const address = region('https://registry-2-dev.iii.cloud')
    expect(address?.querySelector('a')?.getAttribute('href')).toBe('https://registry-2-dev.iii.cloud')
    expect(address?.textContent).toContain('Copy')
    expect(address?.textContent).toContain("iii cloud says it's healthy")
    expect(address?.textContent).toContain('11 of 11 workers up')
    expect(text()).toContain('roll back to #5 in iii cloud')
    await click(btn('Back to dev'))
    expect(path()).toBe('Projects / registry · dev')
  })

  it('failed on the cloud: the cause in one sentence, the explanation, the log lines, the way out', async () => {
    await board('deploy-follow-failed')
    expect(title()).toBe('Deployment #6 stopped while starting the workers')
    expect(text()).toContain('dev still serves #5. Nothing changed for your users.')
    const cause = region('What happened')
    expect(cause?.textContent).toContain('The search worker ran out of memory while starting')
    expect(cause?.textContent).toContain('Machines in the jobs group on dev have 512 MB')
    expect(cause?.textContent).toContain('search: heap 498 MB / 512 MB')
    expect(cause?.querySelector('a')?.getAttribute('href')).toBe('https://cloud.iii.dev/iii/registry/dev/deployments/6')
    expect(cause?.querySelector('a')?.textContent).toContain('Full log in iii cloud')
    expect(btn('Fix with your agent')).toBeTruthy()
  })

  it('Fix with your agent hands the failure to the agent and the screen returns to the check', async () => {
    const h = await board(
      'deploy-follow-failed',
      {},
      { 'iii-cloud::handoff': { id: 'ho-5', marker: '[iii-cloud handoff ho-5]', delivered: false } },
    )
    await click(btn('Fix with your agent'))
    await flush()
    const handoff = h.calls.find((c) => c.id === 'iii-cloud::handoff')
    expect(handoff?.payload).toMatchObject({ kind: 'deploy_fix', deployment_id: 'dep-6' })
    expect(title()).not.toMatch(/stopped/)
    expect(path()).toBe('Projects / registry · dev / Deploy')
  })

  it('arriving at a failed deployment the agent is already fixing shows the check, where the agent is', async () => {
    const failed = boards['deploy-follow-failed'].status
    const fixing = {
      ...failed,
      agent: {
        handoff_id: 'ho-5',
        session_id: 'sess-5',
        state: 'working',
        kind: 'deploy_fix',
        started_at: new Date().toISOString(),
        last_activity_at: new Date().toISOString(),
        steps: 2,
        current: 'Reading the log',
        question: null,
      },
    } as Status
    await mount(fixing, ctx({ at: 'deploy', env: DEV.id, deploy: 'dep-6' }))
    expect(title()).toBe('Your agent is fixing 1')
    expect(region('Your agent')?.textContent).toContain('The search worker ran out of memory while starting')
    expect(btn('Stop')).toBeTruthy()
  })

  it('a deployment iii cloud holds: Confirm deploy, with {check_id, deployment_id}', async () => {
    const held = {
      ...live6,
      status: 'needs_action' as const,
      outcome: 'running' as const,
      finished_at: null,
    }
    const h = await mount(
      { ...boards['deploy-review'].status, deploys: [held, serving5] },
      ctx({ at: 'deploy', env: DEV.id, deploy: 'dep-6' }),
      { 'iii-cloud::deploy': {} },
    )
    expect(title()).toBe('Deployment #6 needs your action')
    expect(text()).toContain('iii cloud holds this deployment until you approve')
    await click(btn('Confirm deploy'))
    expect(h.calls.find((c) => c.id === 'iii-cloud::deploy')?.payload).toEqual({
      check_id: 'chk-dev',
      deployment_id: 'dep-6',
    })
  })

  it('the stages say Deployments, never Deploys', async () => {
    await board('deploy-follow-live')
    expect(text()).not.toMatch(/\bDeploys?\b(?! \w{7} to)/)
  })
})

describe('the review: Change, held deployments, who is serving, what a refusal says', () => {
  const held = { ...live6, status: 'needs_action' as const, outcome: 'running' as const, finished_at: null }
  const withAgent: Status = {
    ...boards['deploy-what'].status,
    agent: { ...boards['deploy-follow-running'].status.agent, handoff_id: 'ho-1', state: 'working' } as never,
  }

  it('a failed stop ends the Change there: nothing replaces the copy the agent works in, and the error shows', async () => {
    const h = await mount(withAgent, ctx(route), {
      'iii-cloud::handoff::stop': () => Promise.reject(Object.assign(new Error('x'), { code: 'NO_COMMITS' })),
    })
    await click(btn('Change'))
    await click(btn('Check this instead'))
    expect(worker(h.calls).map((c) => c.id)).toEqual(['iii-cloud::handoff::stop'])
    // The stop's own error stays on screen (the next call never hides it).
    expect(text()).toContain('Commit something first')
  })

  it('after a Change the old check cannot be deployed: locked until a newer check exists', async () => {
    const base = boards['deploy-review'].status
    const ready = { ...base, check: { ...base.check!, environment_id: DEV.id } }
    // The check after the `use` is refused: the old ready check is all the screen has, and it is of the old source.
    const h = await mount(ready, ctx(route), {
      'iii-cloud::project::use': {},
      'iii-cloud::check': () => Promise.reject(Object.assign(new Error('x'), { code: 'NO_COMMITS' })),
    })
    expect(title()).toBe('Ready to deploy to dev')
    await click(btn('Change'))
    await click(btn('Check this instead'))
    await flush()
    expect(title()).not.toBe('Ready to deploy to dev')
    expect(text()).toContain('Commit something first')
    const deploys = [...el.querySelectorAll('button')].filter((b) => /^Deploy\b/.test(b.textContent ?? ''))
    expect(deploys.every((b) => b.hasAttribute('disabled'))).toBe(true)
    expect(region('You')).toBeNull()
    // A check with any other id is newer, whatever its clock says: Deploy is back.
    await act(async () =>
      h.handlers.get('iii::iii-cloud-ui::events::console-test')?.({
        kind: 'check',
        id: 'chk-after',
        terminal: true,
        summary: '',
        record: { ...ready.check!, id: 'chk-after', checked_at: new Date(Date.now() + 60_000).toISOString() },
      }),
    )
    expect(title()).toBe('Ready to deploy to dev')
    // ...and the check that arrived replaced the kept refusal.
    expect(text()).not.toContain('Commit something first')
  })

  const refusedChange = {
    'iii-cloud::project::use': {},
    'iii-cloud::check': () => Promise.reject(Object.assign(new Error('x'), { code: 'NO_COMMITS' })),
  }
  const stubStorage = () => {
    const store = new Map<string, string>()
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
      removeItem: (k: string) => void store.delete(k),
    })
  }
  const deployButtons = () => [...el.querySelectorAll('button')].filter((b) => /^Deploy\b/.test(b.textContent ?? ''))

  it('a reload after a refused Change keeps the replaced source from being deployed: the worker makes the old check current again', async () => {
    stubStorage()
    const base = boards['deploy-review'].status
    const ready = { ...base, check: { ...base.check!, environment_id: DEV.id } }
    await mount(ready, ctx(route), refusedChange)
    await click(btn('Change'))
    await click(btn('Check this instead'))
    await flush()
    // The page goes away and comes back (a new hub, the same worker status: the old check is dev's check again).
    await act(async () => root.render(null))
    forgetHubMemory()
    await mount(ready, ctx(route, 2), refusedChange)
    expect(title()).not.toBe('Ready to deploy to dev')
    expect(deployButtons().every((b) => b.hasAttribute('disabled'))).toBe(true)
    vi.unstubAllGlobals()
  })

  it("another tab's mark is shown here: a storage event re-reads it, and Deploy goes off", async () => {
    stubStorage()
    const base = boards['deploy-review'].status
    const ready = { ...base, check: { ...base.check!, environment_id: DEV.id } }
    await mount(ready, ctx(route))
    expect(title()).toBe('Ready to deploy to dev')
    // The other tab changed the source: it wrote the mark; this tab is told by the browser.
    localStorage.setItem(`iii-cloud:moved:${DEV.id}`, ready.check.id)
    await act(async () => void window.dispatchEvent(new StorageEvent('storage', { key: `iii-cloud:moved:${DEV.id}` })))
    expect(title()).not.toBe('Ready to deploy to dev')
    expect(deployButtons().every((b) => b.hasAttribute('disabled'))).toBe(true)
    vi.unstubAllGlobals()
  })

  it('opening another environment for a route (a plain use) does not lock a ready check', async () => {
    const base = boards['deploy-review'].status
    const ready = { ...base, check: { ...base.check!, environment_id: DEV.id } }
    const h = fakeHost(ready, { 'iii-cloud::project::use': {} })
    const page = (route: object, id: number) =>
      act(async () => root.render(<CloudPage host={h.host} panelSide="left" tabId="t" {...ctx(route, id)} />))
    await page(route, 1)
    await flush()
    await page({ at: 'env', env: 'e-prod' }, 2)
    await flush()
    await page(route, 3)
    await flush()
    expect(hubOf(h.host).movedFrom(DEV.id)).toBeNull()
    expect(title()).toBe('Ready to deploy to dev')
  })

  it("after a refused Change, Check again checks the compose file that was picked, not the old check's", async () => {
    const h = await board('deploy-review', {}, refusedChange)
    await click(btn('Change'))
    await click(radio('api/worker-compose.staging.yaml'))
    await click(btn('Check this instead'))
    await flush()
    await click(btn('Check again'))
    expect(lastCall(h.calls)).toMatchObject({
      id: 'iii-cloud::check',
      payload: { run: true, manifest: 'api/worker-compose.staging.yaml' },
    })
  })

  it('a refusal shows once and goes when a check arrives: no second copy comes back under the header', async () => {
    const h = await board('deploy-what', {}, { 'iii-cloud::check': refusedChange['iii-cloud::check'] })
    await click(btn('Check'))
    await flush()
    expect(text().match(/Commit something first/g)).toHaveLength(1)
    await act(async () =>
      h.handlers.get('iii::iii-cloud-ui::events::console-test')?.({
        kind: 'check',
        id: 'chk-agent',
        terminal: true,
        summary: '',
        record: { ...boards['deploy-review'].status.check!, id: 'chk-agent', environment_id: DEV.id },
      }),
    )
    expect(text()).not.toContain('Commit something first')
  })

  it('a held push matched to the check by commit alone is not confirmable: no Confirm, it says so, and the action goes to the check', async () => {
    const s = boards['deploy-review'].status
    // No worker marker (no check_id, not approved, from the console), but the deployed commit is the ready check's.
    const byHand = {
      ...held,
      check_id: null,
      approved_by_worker: false,
      from_console: true,
      commit: { sha: s.check!.commit, message: 'pushed by hand' },
    }
    await mount({ ...s, deploys: [byHand, serving5] }, ctx({ ...route, deploy: 'dep-6' }))
    expect(btn('Confirm deploy')).toBeNull()
    // The title and the line under it agree with the notice: it waits in iii cloud, and deploying replaces it.
    expect(title()).toBe('Deployment #6 is waiting for confirmation in iii cloud')
    expect(text()).not.toContain('needs your action')
    expect(text()).toContain('not started from this machine; deploying from the check replaces it')
    expect(text()).toContain('Deployment #6 was not started from this machine')
    expect(text()).toContain('Deploying from the check replaces it.')
    await click(btn('Go to the check'))
    expect(path()).toBe('Projects / registry · dev / Deploy')
    expect(title()).toBe('Ready to deploy to dev')
  })

  it('Confirm deploy is off for every lock: signed out, a deploy-fix hand-off, another deployment running', async () => {
    const s = boards['deploy-review'].status
    const confirm = () => btn('Confirm deploy') as HTMLButtonElement | null
    const at = ctx({ ...route, deploy: 'dep-6' })
    await mount({ ...s, deploys: [held, serving5] }, at, { 'iii-cloud::deploy': {} })
    expect(confirm()?.disabled).toBe(false)
    const signedOut = {
      ...s,
      deploys: [held, serving5],
      check: { ...s.check!, problems: [{ ...s.check!.problems[0], code: 'SIGN_IN_REQUIRED' }] },
    }
    await mount(signedOut, ctx({ ...route, deploy: 'dep-6' }, 2))
    expect(confirm()?.disabled).toBe(true)
    const fixing = { ...s, deploys: [held, serving5], agent: { ...withAgent.agent, kind: 'deploy_fix' } as never }
    await mount(fixing, ctx({ ...route, deploy: 'dep-6' }, 3))
    expect(confirm()?.disabled).toBe(true)
    const other = { ...running6, id: 'dep-9', number: 9 }
    await mount({ ...s, deploys: [other, held, serving5] }, ctx({ ...route, deploy: 'dep-6' }, 4))
    expect(confirm()?.disabled).toBe(true)
  })

  it('the production name dialog disables its own button, and says why, when a lock appears while it is open', async () => {
    const s = boards['deploy-review'].status
    const p = s.project as NonNullable<Status['project']>
    const prod = { ...p, environment: { ...p.environment!, production: true } }
    const h = await mount({ ...s, project: prod, deploys: [held, serving5] }, ctx({ ...route, deploy: 'dep-6' }), {
      'iii-cloud::deploy': {},
    })
    await click(btn('Confirm deploy'))
    const dialog = () => el.querySelector('[role=dialog]')
    await type(dialog()?.querySelector('input') ?? null, 'dev')
    const go = () => [...(dialog()?.querySelectorAll('button') ?? [])].find((b) => b.textContent === 'Confirm deploy')
    expect(go()?.hasAttribute('disabled')).toBe(false)
    // Another deployment of dev starts while the dialog is open.
    await act(async () =>
      h.handlers.get('iii::iii-cloud-ui::events::console-test')?.({
        kind: 'deploy',
        id: 'dep-9',
        terminal: false,
        summary: '',
        record: { ...running6, id: 'dep-9', number: 9 },
      }),
    )
    expect(go()?.hasAttribute('disabled')).toBe(true)
    expect(dialog()?.textContent).toContain('Deployment #9 is still going.')
    expect(h.calls.some((c) => c.id === 'iii-cloud::deploy')).toBe(false)
  })

  it('the Change note names the deployment iii cloud says serves, and none before the list is read', async () => {
    await board('deploy-what', {}, cloudList(serving5))
    await click(btn('Change'))
    expect(region('What goes to dev')?.textContent).toContain('dev serves d738d2f · deployment #5')
    await board('deploy-what', ctx(route, 2), cloudAnswers({}))
    await click(btn('Change'))
    expect(region('What goes to dev')?.textContent).not.toMatch(/serves|Nothing is live/)
  })

  it('the last refused check shows at the top, kept by the hub after the card that asked is gone', async () => {
    const h = await board('deploy-what')
    await act(async () => hubOf(h.host).setCheckError(DEV.id, { code: 'NO_COMMITS', message: 'x' }))
    expect(text()).toContain('Commit something first')
    await act(async () => hubOf(h.host).setCheckError(DEV.id, null))
    expect(text()).not.toContain('Commit something first')
  })

  it('a deployment iii cloud holds on production: Confirm asks for the name first, then sends check_id and deployment_id', async () => {
    const s = boards['deploy-review'].status
    const p = s.project as NonNullable<Status['project']>
    const prod = { ...p, environment: { ...p.environment!, production: true } }
    const h = await mount({ ...s, project: prod, deploys: [held, serving5] }, ctx({ ...route, deploy: 'dep-6' }), {
      'iii-cloud::deploy': {},
    })
    await click(btn('Confirm deploy'))
    expect(h.calls.some((c) => c.id === 'iii-cloud::deploy')).toBe(false)
    const dialog = el.querySelector('[role=dialog]')
    expect(dialog).toBeTruthy()
    await type(dialog?.querySelector('input') ?? null, 'dev')
    await click([...(dialog?.querySelectorAll('button') ?? [])].find((b) => b.textContent === 'Confirm deploy'))
    expect(h.calls.find((c) => c.id === 'iii-cloud::deploy')?.payload).toEqual({
      check_id: 'chk-dev',
      deployment_id: 'dep-6',
    })
  })

  it('Cancel the deployment is not offered for one that did not start on this machine', async () => {
    const other = { ...running6, check_id: null, approved_by_worker: false, from_console: true }
    const s = boards['deploy-follow-running'].status
    await mount({ ...s, deploys: [other, serving5] }, ctx({ ...route, deploy: 'dep-6' }))
    expect(title()).toBe('Deploying 9c04e7f to dev')
    expect(btn('Cancel the deployment')).toBeNull()
  })

  it('a replaced deployment is not "serving" because its outcome says ready: iii cloud\'s list decides', async () => {
    const s = boards['deploy-follow-live'].status
    await mount({ ...s, deploys: [live6, serving5] }, ctx({ ...route, deploy: 'dep-5' }))
    expect(title()).not.toMatch(/is serving/)
  })

  it('a deleted project or environment shows what the environment page shows', async () => {
    const s = boards['deploy-what'].status
    await mount({ ...s, problems: [{ code: 'ENVIRONMENT_DELETED', message: 'gone' }] }, ctx(route))
    expect(text()).toContain('dev was deleted in iii cloud')
    expect(text()).not.toContain('Check feat/search-rank')
  })

  it('Change takes focus when it opens and gives it back when it closes; radios carry aria-checked only', async () => {
    await board('deploy-what')
    const trigger = btn('Change')
    await click(trigger)
    expect(document.activeElement?.classList.contains('iii-cloud-cs-change')).toBe(true)
    expect(el.querySelectorAll('[role=radio][aria-pressed]')).toHaveLength(0)
    await click(btn('Cancel'))
    expect(document.activeElement).toBe(btn('Change'))
  })
})

describe('a narrow pane', () => {
  it('the failed deployment: one column, Fix with your agent in the bar at the bottom', async () => {
    const { ui } = await import('./util')
    ui.narrow = true
    await board('deploy-follow-failed')
    expect(el.querySelector('.iii-cloud-cs-bar')?.textContent).toContain('Fix with your agent')
    expect(el.querySelector('.iii-cloud-stephead-fwd')).toBeNull()
  })
})
