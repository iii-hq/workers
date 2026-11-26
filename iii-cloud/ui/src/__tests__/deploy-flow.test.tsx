// The Deploy screen (DeployArea): one screen driven by state, no step rail. The check and Ready are CheckStage's
// (its own suite); here: opening with no check, Change, the deployment (stages, live, failed on the cloud, held), and
// the way out of each.
import { describe, expect, it, vi } from 'vitest'
import { boards } from '../fixtures'
import { DEV, live6, running6, serving5 } from '../fixtures/deploy'
import type { BranchRef, Status } from '../types'
import type { Call } from './host'
import { board, btn, click, ctx, el, flush, lastCall, mount, text, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

const worker = (calls: Call[]) => calls.filter((c) => c.id !== 'iii-cloud::status' && c.id !== 'iii-cloud::values')
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
    const h = await board('deploy-what')
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

describe('a narrow pane', () => {
  it('the failed deployment: one column, Fix with your agent in the bar at the bottom', async () => {
    const { ui } = await import('./util')
    ui.narrow = true
    await board('deploy-follow-failed')
    expect(el.querySelector('.iii-cloud-cs-bar')?.textContent).toContain('Fix with your agent')
    expect(el.querySelector('.iii-cloud-stephead-fwd')).toBeNull()
  })
})
