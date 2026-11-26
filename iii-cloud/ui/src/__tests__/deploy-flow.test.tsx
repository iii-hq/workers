// Deploy from this machine (U4): Dep1 what goes, Dep5 review and deploy, Dep6 follow; the check step is CheckStage's.
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { boards } from '../fixtures'
import { agentStatus, base, ENV, following, pr, ready } from '../fixtures/base'
import { DEV, devCheck, running6, serving5 } from '../fixtures/deploy'
import { devOptional, devValues } from '../fixtures/values'
import type { BranchRef, Status } from '../types'
import type { Call } from './host'
import { board, btn, byText, click, ctx, el, flush, lastCall, mount, text, type, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

const fail = (code: string, message = 'refused') => Object.assign(new Error(message), { code })
/** The page's calls, without its reads of the status and of the values. */
const worker = (calls: Call[]) => calls.filter((c) => c.id !== 'iii-cloud::status' && c.id !== 'iii-cloud::values')
const current = () => el.querySelector('[aria-current=step]')?.textContent ?? ''
const radio = (label: string) =>
  [...el.querySelectorAll('[role=radio]')].find((r) => r.textContent?.startsWith(label)) as HTMLElement | undefined
const dialogButton = (t: string) => [...el.querySelectorAll('[role=dialog] button')].find((b) => b.textContent === t)
const route = (r: Record<string, unknown>, id = 1) => ctx({ at: 'deploy', ...r }, id)
/** production's check passed; with its pull request open; after the merge, its deploy running. */
const passedProd: Status = { ...base, check: ready }
const withPr: Status = { ...passedProd, pr }
const merged: Status = { ...passedProd, pr: { ...pr, state: 'MERGED' }, deploys: [following] }

describe('copy', () => {
  it.each([
    'deploy-what',
    'deploy-review',
    'deploy-follow-running',
    'deploy-follow-live',
    'deploy-follow-failed',
  ] as const)('%s never claims nothing is pushed, and offers no deferred action', async (id) => {
    await board(id)
    expect(text()).not.toMatch(/nothing is pushed|nothing reaches iii cloud/i)
    expect(text()).not.toContain('Deploy mine after it')
    expect(text()).not.toContain("Your branch has the agent's")
    expect(text()).not.toContain("Bring your agent's")
    expect(text()).not.toContain('Give jobs')
  })
})

describe('P1: the forward action top right in the step header, and none below', () => {
  const fwd = () => [...el.querySelectorAll('.iii-cloud-stephead-fwd button')].map((b) => b.textContent)
  it('Dep1: Check before deploy, Cancel left of it, the note under the title', async () => {
    await board('deploy-what')
    expect(fwd()).toEqual(['Check before deploy'])
    expect(el.querySelector('.iii-cloud-stephead-back')?.textContent).toBe('Cancel')
    expect(el.querySelector('.iii-cloud-stephead-reason')?.textContent).toBe(
      'Nothing goes live before you confirm. Every deploy from this machine is checked first.',
    )
    expect(el.querySelector('.iii-cloud-dep-actions')).toBeNull()
  })

  it('Dep5: Deploy, with the chip next to the title', async () => {
    await board('deploy-review')
    expect(fwd()).toEqual(['Deploy 9c04e7f to dev'])
    expect(el.querySelector('.iii-cloud-stephead-title')?.textContent).toBe('Ready to deploy to devPassed')
    expect(el.querySelector('.iii-cloud-dep-actions')).toBeNull()
  })
})

describe('Dep1 what goes', () => {
  it('compares the folder with what dev serves: the serving deploy, the commits dev lacks, the radios', async () => {
    await board('deploy-what')
    expect(text()).toContain('What goes to dev?')
    expect(text()).toContain('d738d2f Drop cloud.web · deploy #5')
    expect(text()).toContain("feat/search-rank · 3 commits dev doesn't have")
    const listed = [...el.querySelectorAll(`[aria-label="Commits dev doesn't have"] li`)].map((l) => l.textContent)
    expect(listed).toEqual([
      'a41c09e Search: rank by stars',
      '7b2e5d1 Add trigram index for search',
      'c90f114 Billing worker for paid kits',
    ])
    expect(radio('dev')?.getAttribute('aria-checked')).toBe('true')
    expect(radio('dev')?.textContent).toContain('pushes, then deploys')
    expect(radio('production')?.getAttribute('aria-checked')).toBe('false')
    expect(radio('production')?.textContent).toContain('opens a pull request')
    expect(text()).toContain('dev deploys whatever is on dev')
    expect(text()).toContain('Nothing goes live before you confirm.')
    expect(current()).toContain('What goes')
  })

  it('never offers a .env: it says it stays, and offers only the other changed files', async () => {
    await board('deploy-what')
    expect(text()).toContain('2 files changed · api/src/search.ts, api/migrations/0008_trgm.sql')
    expect(text()).toContain('Not brought: .env — secrets stay on this machine.')
  })

  it('Check before deploy: project::use from the checkout with the changes, then check {run, push}, then the check step', async () => {
    const h = await board('deploy-what')
    await click(btn('Check before deploy'))
    const calls = worker(h.calls)
    expect(calls.map((c) => c.id)).toEqual(['iii-cloud::project::use', 'iii-cloud::check'])
    expect(calls[0]).toEqual({
      id: 'iii-cloud::project::use',
      payload: {
        path: '/home/you/workspaces/registry',
        environment_id: DEV.id,
        mode: 'worktree',
        from: 'checkout',
        bring_changes: true,
      },
    })
    expect(calls[1].payload).toEqual({ run: true, push: true })
    expect(current()).toContain('Check before deploy')
  })

  it('the picker: the file goes into the check as manifest; one iii cloud cannot read is disabled', async () => {
    const h = await board('deploy-what')
    expect(text()).toContain('api/worker-compose-prod.yaml· the file dev runs, set on the environment')
    await click(btn('Change'))
    expect(btn('Done')?.getAttribute('aria-expanded')).toBe('true')
    expect(el.querySelectorAll('[aria-label="Compose files in this folder"] [role=radio]')).toHaveLength(3)
    expect(radio('api/workers-compose.yaml')?.getAttribute('aria-disabled')).toBe('true')
    expect(radio('api/workers-compose.yaml')?.textContent).toContain('not a worker-compose*.yaml')
    expect(radio('api/worker-compose-prod.yaml')?.textContent).toContain('11 workers · what dev runs now')
    await click(radio('api/worker-compose.staging.yaml'))
    expect(text()).toContain('· changes with this deploy')
    await click(el.querySelector('input[type=checkbox]'))
    await click(btn('Check before deploy'))
    expect(worker(h.calls)[0].payload?.bring_changes).toBe(false)
    expect(lastCall(h.calls)).toEqual({
      id: 'iii-cloud::check',
      payload: { run: true, push: true, manifest: 'api/worker-compose.staging.yaml' },
    })
  })

  it("Values: how many names the folder's file reads are not set in dev, read from the checkout with the pick", async () => {
    const h = await board('deploy-what')
    const values = () => h.calls.filter((c) => c.id === 'iii-cloud::values').map((c) => c.payload)
    expect(values()).toEqual([{ manifest: 'api/worker-compose-prod.yaml', from: 'checkout' }])
    const valuesRow = () =>
      [...el.querySelectorAll('.iii-cloud-dep-row')].find((r) => r.textContent?.startsWith('Values'))
    // Only the required one is named; the optional ones default to empty.
    expect(valuesRow()?.textContent).toBe(
      "Values1 required name your folder's api/worker-compose-prod.yaml reads isn't set in dev: STRIPE_SECRET_KEYSet values",
    )
    await click(btn('Change'))
    await click(radio('api/worker-compose.staging.yaml'))
    expect(values().at(-1)).toEqual({ manifest: 'api/worker-compose.staging.yaml', from: 'checkout' })
  })

  it('Values: only optional names unset reads neutral and faint, Set values secondary', async () => {
    await board('deploy-what-optional')
    const valuesRow = [...el.querySelectorAll('.iii-cloud-dep-row')].find((r) => r.textContent?.startsWith('Values'))
    const said = valuesRow?.querySelector('.iii-cloud-faint')
    expect(said?.textContent).toBe('2 optional names have no value in dev; they default to empty')
    expect(valuesRow?.textContent).not.toContain("aren't set")
    expect(btn('Set values')?.getAttribute('data-variant')).toBe('secondary')
  })

  it('Values (SKIP-VALUES): the skipped ones are counted in, then said: "…; 1 skipped"', async () => {
    const one = { ...devOptional, items: devOptional.items.map((i, n) => (n === 1 ? { ...i, skipped: true } : i)) }
    await board('deploy-what-optional', {}, { 'iii-cloud::values': one })
    const valuesRow = [...el.querySelectorAll('.iii-cloud-dep-row')].find((r) => r.textContent?.startsWith('Values'))
    expect(valuesRow?.querySelector('.iii-cloud-faint')?.textContent).toBe(
      '2 optional names have no value in dev; 1 skipped',
    )
  })

  it('Set values opens the dialog on the same file; Save writes, closes, and the row reads again; no check', async () => {
    const saved = { environment_id: DEV.id, results: [{ name: 'SEARCH_WEIGHT_STARS', ok: true }] }
    const h = await board('deploy-what', {}, { 'iii-cloud::values::save': saved })
    await click(btn('Set values'))
    const dialog = el.querySelector('[role=dialog]')
    expect(dialog?.querySelector('h2')?.textContent).toBe('Values for dev')
    expect(h.calls.filter((c) => c.id === 'iii-cloud::values')).toHaveLength(2)
    await click(btn('Save 1 value'))
    await flush()
    expect(h.calls.find((c) => c.id === 'iii-cloud::values::save')?.payload).toEqual({
      environment_id: DEV.id,
      manifest: 'api/worker-compose-prod.yaml',
      items: [{ name: 'SEARCH_WEIGHT_STARS', from: 'env_file', type: 'variable', file: '.env' }],
    })
    expect(el.querySelector('[role=dialog]')).toBeNull()
    expect(h.calls.filter((c) => c.id === 'iii-cloud::values')).toHaveLength(3)
    expect(h.calls.some((c) => c.id === 'iii-cloud::check')).toBe(false)
  })

  it('nothing unset: "All 4 are set in dev", no Set values; hidden while dev is being selected', async () => {
    const all = { ...devValues, items: devValues.items.map((i) => ({ ...i, in_cloud: 'set' as const })) }
    await board('deploy-what', {}, { 'iii-cloud::values': all })
    expect(text()).toContain('All 4 are set in dev')
    expect(btn('Set values')).toBeNull()
    const s = boards['deploy-what'].status
    const p = s.project as NonNullable<Status['project']>
    await mount(
      { ...s, project: { ...p, environments: [...p.environments, base.project!.environment!] } },
      route({ env: ENV.id }, 2),
      {
        'iii-cloud::values': all,
      },
    )
    expect(text()).not.toContain('are set in')
  })

  it('production starts from the environment, never the checkout, and says your commits are not in it', async () => {
    const s = boards['deploy-what'].status
    const p = s.project as NonNullable<Status['project']>
    const prod = { ...p, environment: { ...base.project!.environment!, url: null } }
    const h = await mount({ ...s, project: prod }, route({ env: prod.environment.id }))
    expect(text()).toContain("They aren't in a production check: it starts from main")
    expect(text()).toContain('production deploys what is merged into main')
    await click(btn('Check before deploy'))
    expect(worker(h.calls)[0].payload?.from).toBe('environment')
  })

  it('a radio picks another environment; New environment opens with this branch', async () => {
    await board('deploy-what')
    await click(radio('production'))
    expect(text()).toContain('What goes to production?')
    await board('deploy-what')
    await click(radio('New environment'))
    expect(el.querySelector('aside[aria-label="Deploy steps"]')).toBeNull()
  })

  it('MERGE_CONFLICT: nothing is checked; the agent merges your branch in the worktree, as a follow-up', async () => {
    const s = { ...boards['deploy-what'].status, check: devCheck }
    const h = await mount(s, route({ env: DEV.id, step: 'what' }), {
      'iii-cloud::project::use': fail(
        'MERGE_CONFLICT',
        'The commits of feat/search-rank conflict with dev: your agent can merge feat/search-rank in the worktree /w and resolve them. Conflicting files: src/rank.ts.',
      ),
      'iii-cloud::handoff': { id: 'h1', marker: 'iii-cloud hand-off h1', delivered: false, reused: false },
    })
    await click(btn('Check before deploy'))
    const p = el.querySelector('[role=status][data-variant=warn]')?.textContent
    expect(p).toContain("Your commits conflict with the environment's branch")
    expect(p).toContain('Nothing was checked.')
    expect(h.calls.some((c) => c.id === 'iii-cloud::check')).toBe(false)
    expect(current()).toContain('What goes')
    await click(btn('Ask your agent to bring your commits in'))
    const ask = h.calls.find((c) => c.id === 'iii-cloud::handoff')
    expect(ask?.payload).toMatchObject({ kind: 'ready', check_id: 'chk-dev' })
    expect(String(ask?.payload?.text)).toContain(
      'Merge feat/search-rank into the worktree and resolve the conflicts in src/rank.ts, then check again.',
    )
    expect(h.compose).toHaveBeenCalledWith(
      expect.objectContaining({ text: expect.stringMatching(/^iii-cloud hand-off h1\nMerge feat\/search-rank/) }),
    )
  })

  it('MERGE_IN_PROGRESS: the worker says why, and the agent is asked to finish the open merge', async () => {
    const s = { ...boards['deploy-what'].status, check: devCheck }
    const h = await mount(s, route({ env: DEV.id, step: 'what' }), {
      'iii-cloud::project::use': fail('MERGE_IN_PROGRESS', 'A merge is still open in the worktree /w.'),
      'iii-cloud::handoff': { id: 'h2', marker: 'iii-cloud hand-off h2', delivered: false, reused: false },
    })
    await click(btn('Check before deploy'))
    const p = el.querySelector('[role=status][data-variant=warn]')?.textContent
    expect(p).toContain('A merge is still open in the worktree /w.')
    expect(p).toContain('Nothing was checked.')
    expect(h.calls.some((c) => c.id === 'iii-cloud::check')).toBe(false)
    await click(btn('Ask your agent to finish the merge'))
    const ask = h.calls.find((c) => c.id === 'iii-cloud::handoff')
    expect(ask?.payload).toMatchObject({ kind: 'ready', check_id: 'chk-dev' })
    expect(String(ask?.payload?.text)).toContain('Finish the merge in the worktree (commit it), then check again.')
  })

  it('pushes_directly decides the direct case; without a default branch nothing claims a push', async () => {
    const s = passedProd
    const brief = { ...agentStatus.handoff_brief!, pushes_directly: true }
    await mount({ ...s, pr: null, handoff_brief: brief }, route({ env: '' }))
    expect(radio('production')?.textContent).toContain('pushes, then deploys')
    await mount(s, route({ env: '' }, 2))
    expect(radio('production')?.textContent).toContain('opens a pull request')
    expect(text()).not.toContain('pushes, then deploys')
  })
})

describe('the check step', () => {
  it('says which changed files stayed on this machine', async () => {
    const s = boards['deploy-review'].status
    const p = s.project as NonNullable<Status['project']>
    const workspace = { ...p.workspace!, not_brought: ['.env'] }
    await mount({ ...s, check: { ...devCheck, ready: false } }, route({ env: DEV.id, step: 'check' }))
    expect(text()).not.toContain('Not brought')
    await mount(
      { ...s, project: { ...p, workspace }, check: { ...devCheck, ready: false } },
      route({ env: DEV.id, step: 'check' }, 2),
    )
    expect(text()).toContain('Not brought: .env — secrets stay on this machine.')
    expect(current()).toContain('Check before deploy')
  })

  it('a review asked before the check passed shows the check', async () => {
    const s = boards['deploy-review'].status
    await mount({ ...s, check: { ...devCheck, ready: false } }, route({ env: DEV.id, step: 'review' }))
    expect(current()).toContain('Check before deploy')
    expect(text()).not.toContain('Ready to deploy')
  })
})

describe('Dep5 review and deploy', () => {
  it('rows: commits split at from_head, the app file, the release, secrets, settings and how it ships', async () => {
    await board('deploy-review')
    expect(text()).toContain('Ready to deploy to dev')
    expect(text()).toMatch(/Commit 9c04e7f passed on iii cloud and here at .+ · 3 checks, 9 min/)
    expect(text()).toContain("6 commits dev doesn't have4 of yours, 2 by your agent")
    expect(byText('a', 'See changes')?.getAttribute('href')).toBe(
      `https://github.com/iii-hq/registry/compare/${'d738d2f'.padEnd(40, '0')}...${'9c04e7f'.padEnd(40, '0')}`,
    )
    expect(text()).toContain('api/worker-compose-prod.yaml11 workers in 3 groups')
    expect(byText('a', 'See the file')?.getAttribute('href')).toContain('/blob/9c04e7f')
    expect(text()).toContain('Release command db:migrate runs before the new version starts')
    expect(text()).toContain('1 new: STRIPE_SECRET_KEY, saved by you')
    expect(text()).toContain('Console settingsNo change')
    expect(text()).toContain('Pushes 9c04e7f to dev, then starts a deploy')
    expect(text()).toContain('dev serves #5 until the new deploy is live.')
    expect(text()).toContain("Only you can confirm this; your agent can't.")
    expect(el.querySelector('input[type=checkbox]')).toBeNull()
    expect(current()).toContain('Review and deploy')
  })

  it('a direct environment that is not production deploys on the click: check_id only, then follows', async () => {
    const h = await board('deploy-review', {}, { 'iii-cloud::deploy': running6 })
    await click(btn('Deploy 9c04e7f to dev'))
    expect(el.querySelector('[role=dialog]')).toBeNull()
    expect(h.calls.find((c) => c.id === 'iii-cloud::deploy')?.payload).toEqual({ check_id: 'chk-dev' })
    expect(current()).toContain('Live')
  })

  it('a double click on Deploy sends one deploy: the button stays disabled while it runs', async () => {
    const h = await board('deploy-review', {}, { 'iii-cloud::deploy': () => new Promise(() => undefined) })
    await click(btn('Deploy 9c04e7f to dev'))
    await click(btn('Deploy 9c04e7f to dev'))
    expect(h.calls.filter((c) => c.id === 'iii-cloud::deploy')).toHaveLength(1)
    expect(btn('Deploy 9c04e7f to dev')?.hasAttribute('disabled')).toBe(true)
  })

  it("production's typed confirm stays open with its confirm disabled while the deploy runs: one call", async () => {
    const h = await mount(withPr, ctx({ tab: 'deploys', pr: true }), {
      'iii-cloud::deploy': () => new Promise(() => undefined),
    })
    await click(btn('Merge to main and deploy to production…'))
    await type(el.querySelector('[role=dialog] input'), 'production')
    await click(dialogButton('Merge and deploy'))
    expect(dialogButton('Merge and deploy')?.hasAttribute('disabled')).toBe(true)
    await click(dialogButton('Merge and deploy'))
    expect(h.calls.filter((c) => c.id === 'iii-cloud::deploy')).toHaveLength(1)
    expect(btn('Merge to main and deploy to production…')?.hasAttribute('disabled')).toBe(true)
  })

  it('a deploy of dev still running locks Review, says so, and Follow #6 follows it', async () => {
    const s = boards['deploy-review'].status
    await mount({ ...s, deploys: [running6, ...s.deploys] }, route({ env: DEV.id, step: 'review' }))
    expect(text()).toContain('Deploy #6 to dev is already running')
    expect(btn('Deploy 9c04e7f to dev')?.hasAttribute('disabled')).toBe(true)
    await click(btn('Follow #6'))
    expect(text()).toContain('Deploying 9c04e7f to dev')
    expect(current()).toContain('Live')
  })

  it('an environment on another branch says it pushes there, and deploys with check_id only', async () => {
    const s = passedProd
    const staging = { id: 'env-stg', name: 'staging', production: false, branch: 'develop' }
    const project = {
      ...s.project!,
      default_branch: 'main',
      environment: { ...s.project!.environment!, ...staging },
      environments: [...s.project!.environments, staging],
    }
    const h = await mount(
      { ...s, project, check: { ...s.check!, environment_id: 'env-stg' }, pr: null },
      route({ env: 'env-stg', step: 'review' }),
      { 'iii-cloud::deploy': () => following },
    )
    expect(text()).toContain('Pushes 3c4d2e0 to develop, then starts a deploy')
    expect(text()).not.toContain('pull request')
    await click(btn('Deploy 3c4d2e0 to staging'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::deploy', payload: { check_id: 'chk-ready' } })
  })

  it('production: the pull request facts, then the typed name before Merge and deploy {check_id, pr}', async () => {
    const h = await mount(withPr, ctx({ tab: 'deploys', pr: true }), { 'iii-cloud::deploy': () => following })
    expect(text()).toContain('Merges pull request #12, Make registry ready for iii cloud, into main, then deploys')
    expect(text()).toContain('iii-cloud/ready-production → main, opened by your agent, with your GitHub account.')
    expect(text()).toContain('GitHub checks: Running. main has not moved since the check.')
    expect(text()).toContain('Pinned to the plan the check saw')
    expect(byText('a', 'Open on GitHub')?.getAttribute('href')).toBe('https://github.com/iii-hq/registry/pull/12')
    expect(text()).toContain('Creates a shared Postgres for DATABASE_URL')
    expect(text()).toContain('Approving resources in production takes Owner or Admin. You are Owner.')
    expect(text()).toContain('Changes 3 settings the console had')
    expect(text()).toContain('Web, healthcheck, release command now come from api/worker-compose-prod.yaml.')
    await click(btn('Merge to main and deploy to production…'))
    const d = el.querySelector('[role=dialog]')?.textContent ?? ''
    expect(d).toContain('Merge pull request #12 to main and deploy registry to production?')
    expect(d).toContain('This merges pull request #12 (iii-cloud/ready-production) into main.')
    expect(dialogButton('Merge and deploy')?.hasAttribute('disabled')).toBe(true)
    await type(el.querySelector('[role=dialog] input'), 'prod')
    expect(dialogButton('Merge and deploy')?.hasAttribute('disabled')).toBe(true)
    expect(h.calls.some((c) => c.id === 'iii-cloud::deploy')).toBe(false)
    await type(el.querySelector('[role=dialog] input'), 'production')
    await click(dialogButton('Merge and deploy'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::deploy', payload: { check_id: 'chk-ready', pr: 12 } })
    expect(current()).toContain('Live')
  })

  it('says what iii cloud creates only when the plan has a Postgres', async () => {
    const s = withPr
    const none = { ...s.check!, plan: { ...s.check!.plan, confirmations: { resources: [], overrides: [] } } }
    await mount({ ...s, check: none }, route({ env: '', step: 'review' }))
    expect(text()).not.toContain('Postgres')
    expect(text()).toContain('Console settingsNo change')
  })

  it('a moved main locks Merge and deploy and checks again; a closed pull request asks the agent for one', async () => {
    const s = withPr
    const h = await mount({ ...s, pr: { ...s.pr!, main_moved: true } }, ctx({ tab: 'deploys', pr: true }))
    expect(text()).toContain('main moved since the check')
    expect(btn('Merge to main and deploy to production…')?.hasAttribute('disabled')).toBe(true)
    await click(btn('Check again'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
    const h2 = await mount({ ...s, pr: { ...s.pr!, state: 'CLOSED' } }, ctx({ tab: 'deploys', pr: true }, 2))
    expect(btn('Merge to main and deploy to production…')?.hasAttribute('disabled')).toBe(true)
    await click(btn('Ask your agent to open it'))
    expect(h2.calls.find((c) => c.id === 'iii-cloud::handoff')?.payload).toMatchObject({ kind: 'pull_request' })
  })

  it.each([
    'NOT_READY',
    'MAIN_MOVED',
    'PLAN_CHANGED',
  ])('%s locks Deploy and offers Check again, which checks the same file again', async (code) => {
    const h = await board('deploy-review', {}, { 'iii-cloud::deploy': fail(code, 'The check moved on.') })
    await click(btn('Deploy 9c04e7f to dev'))
    const again = btn('Check again')
    expect(again?.getAttribute('data-variant')).toBe('primary')
    expect(btn('Deploy 9c04e7f to dev')?.hasAttribute('disabled')).toBe(true)
    if (code === 'MAIN_MOVED') expect(text()).toContain('The base branch moved after the check')
    await click(again)
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
    expect(current()).toContain('Check before deploy')
  })

  it('PUSH_UNKNOWN: a plain sentence without git output, See Deploys, and Deploy stays locked', async () => {
    const message =
      'Could not tell whether 9c04e7f reached dev: the push failed and origin did not answer after. See Deploys before deploying again. Git said: fatal: unable to access'
    await board('deploy-review', {}, { 'iii-cloud::deploy': fail('PUSH_UNKNOWN', message) })
    await click(btn('Deploy 9c04e7f to dev'))
    expect(text()).toContain("Couldn't tell whether the push reached GitHub.")
    expect(text()).toContain('See Deploys before you deploy again.')
    expect(text()).not.toContain('fatal:')
    expect(btn('Deploy again')).toBeFalsy()
    expect(btn('Deploy 9c04e7f to dev')?.hasAttribute('disabled')).toBe(true)
    await click(btn('See Deploys'))
    expect(el.querySelector('aside[aria-label="Deploy steps"]')).toBeNull()
  })

  it('another app file: the review names it and its switch', async () => {
    const s = withPr
    const check = { ...s.check!, manifest: 'api/worker-compose-next.yaml' }
    const h = await mount({ ...s, check, pr: { ...s.pr!, main_moved: true } }, route({ env: '', step: 'review' }))
    expect(text()).toContain('api/worker-compose-next.yaml')
    expect(text()).toContain('production switches to it when you confirm the deploy.')
    expect(text()).toContain('now come from api/worker-compose-next.yaml')
    await click(btn('Check again'))
    expect(lastCall(h.calls)).toEqual({
      id: 'iii-cloud::check',
      payload: { run: true, push: true, manifest: 'api/worker-compose-next.yaml' },
    })
  })
})

describe('Dep6 follow', () => {
  const rows = () => [...el.querySelectorAll('[aria-label="Deploy stages"] [role=row]')].map((r) => r.textContent ?? '')

  it('running: it keeps going if you switch projects, said once under the header; not once it ended', async () => {
    await board('deploy-follow-running')
    const note = 'It keeps going if you switch projects; the pill at the top brings you back.'
    expect(text().split(note)).toHaveLength(2)
    await board('deploy-follow-live')
    expect(text()).not.toContain(note)
  })

  it('running: the stages with the release row, the live log from the cloud, and dev keeps serving #5', async () => {
    const h = await board('deploy-follow-running')
    expect(text()).toContain('Deploying 9c04e7f to dev')
    expect(text()).toMatch(/Deploy #6 · started .+ · \d+ min so far/)
    expect(el.querySelector('[aria-live=polite]')?.textContent).toBe('Deploying')
    const labels = rows()
    expect(labels).toHaveLength(7)
    expect(labels[4]).toContain('Release command')
    expect(labels[5]).toContain('Workers: 8 of 11 ready')
    expect(el.querySelector('[data-terminal="Live log"]')?.textContent).toBe(
      'applying migration 0008_trgm.sql · done\nbilling: started in group app\nsearch: loading trigram index (1.2M rows)',
    )
    expect(h.calls.find((c) => c.id === 'iii-cloud::api')?.payload).toEqual({
      method: 'GET',
      path: `/teams/5b0d6f2e-0c1a-4e3b-9a77-1d2c3b4a5f60/environments/${DEV.id}/logs`,
      query: { deployment_id: 'dep-6' },
    })
    expect(text()).toContain('dev keeps serving #5 until #6 is live.')
    expect(byText('a', 'Open in iii cloud')?.getAttribute('href')).toContain('/dev/deployments/6')
    expect(current()).toContain('Live')
  })

  it('live: the address, iii cloud health, and the way back; roll back is under Deploys', async () => {
    await board('deploy-follow-live')
    expect(text()).toContain('dev is serving 9c04e7f')
    expect(text()).toMatch(/Deploy #6 · live at .+ · took \d+ min/)
    expect(text()).toContain('https://registry-2-dev.iii.cloud')
    expect(text()).toContain("iii cloud says it's healthy")
    expect(text()).toContain('11 of 11 workers up')
    expect(text()).toContain("If something's off, roll back to #5 under Deploys.")
    await click(btn('Back to dev'))
    expect(el.querySelector('aside[aria-label="Deploy steps"]')).toBeNull()
    expect(el.querySelector('[role=tab][aria-selected=true]')?.textContent).toBe('Overview')
  })

  it('failed: what only the cloud saw, its log lines, and Fix with your agent hands off deploy_fix', async () => {
    const h = await board('deploy-follow-failed')
    expect(text()).toContain('Deploy #6 failed at Workers')
    expect(text()).toContain('dev still serves #5 · nothing changed for your users')
    expect(text()).toContain('Something only the cloud could see')
    expect(text()).toContain('the check here passed on 9c04e7f')
    expect(text()).toContain('The search worker ran out of memory while starting')
    expect(el.querySelector('[data-terminal=log]')?.textContent).toContain('machine jobs-1 stopped: out of memory')
    await click(btn('Fix with your agent'))
    expect(h.calls.find((c) => c.id === 'iii-cloud::handoff')?.payload).toMatchObject({
      kind: 'deploy_fix',
      deployment_id: 'dep-6',
      check_id: 'chk-dev',
    })
    expect(current()).toContain('Check before deploy')
  })

  it('following a merge: the six stages plus the release row, and the link to iii cloud', async () => {
    const f = merged
    await mount(f, route({ env: '', step: 'follow', deploy: f.deploys[0].id }))
    expect(text()).toContain('Deploying 7a1f3b2 to production')
    const labels = rows()
    expect(labels).toHaveLength(7)
    expect(labels[0]).toContain('Validation')
    expect(labels[4]).toContain('Release command')
    expect(labels[5]).toContain('2 of 3 ready')
    expect(labels[6]).toContain('Routes and healthcheck')
    expect(byText('a', 'Open in iii cloud')?.getAttribute('href')).toContain('/deployments/41')
  })

  it('a deploy iii cloud holds: Confirm deploy, the typed name in production, then {check_id, deployment_id}', async () => {
    const s = merged
    const d = { ...s.deploys[0], status: 'needs_action' as const, outcome: 'running' as const, check_id: 'chk-ready' }
    const h = await mount({ ...s, deploys: [d] }, route({ env: '', step: 'follow', deploy: d.id }))
    expect(text()).toContain('Needs your action')
    expect(text()).toContain('a shared Postgres for DATABASE_URL, 3 settings from the app file')
    await click(btn('Confirm deploy'))
    await type(el.querySelector('[role=dialog] input'), 'production')
    await click(el.querySelector('[role=dialog] button[data-variant=primary]'))
    expect(lastCall(h.calls)).toEqual({
      id: 'iii-cloud::deploy',
      payload: { check_id: 'chk-ready', deployment_id: d.id },
    })
  })
  it('a deploy held for an older check than the passed one: Confirm deploy is disabled, says why, Check again', async () => {
    const s = boards['deploy-review'].status
    const held = { ...running6, status: 'needs_action' as const, check_id: 'chk-1' }
    const h = await mount({ ...s, deploys: [held, serving5] }, route({ env: DEV.id, step: 'follow', deploy: held.id }))
    expect(text()).toContain('Needs your action')
    expect(btn('Confirm deploy')?.hasAttribute('disabled')).toBe(true)
    expect(text()).toContain('No passed check here matches deploy #6. Check again before you deploy.')
    await click(btn('Confirm deploy'))
    expect(h.calls.some((c) => c.id === 'iii-cloud::deploy')).toBe(false)
    await click(btn('Check again'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
  })

  it("a held deploy without check_id pairs with the check only on the check's commit", async () => {
    const s = boards['deploy-review'].status
    const held = { ...running6, status: 'needs_action' as const, check_id: null }
    const h = await mount({ ...s, deploys: [held, serving5] }, route({ env: DEV.id, step: 'follow', deploy: held.id }))
    await click(btn('Confirm deploy'))
    expect(lastCall(h.calls)).toEqual({
      id: 'iii-cloud::deploy',
      payload: { check_id: 'chk-dev', deployment_id: held.id },
    })
    const other = { ...held, commit: serving5.commit }
    await mount({ ...s, deploys: [other, serving5] }, route({ env: DEV.id, step: 'follow', deploy: other.id }, 2))
    expect(btn('Confirm deploy')?.hasAttribute('disabled')).toBe(true)
  })

  it("a deploy of another environment isn't followed here: Deploy not found", async () => {
    const s = boards['deploy-review'].status
    await mount({ ...s, deploys: [following, serving5] }, route({ env: DEV.id, step: 'follow', deploy: following.id }))
    expect(text()).toContain('Deploy not found')
    expect(text()).not.toContain('7a1f3b2')
  })
})

describe('the way out of Deploy', () => {
  const nav = () => el.querySelector('[role=toolbar] nav')?.textContent
  const selectedTab = () => el.querySelector('[role=tab][aria-selected=true]')?.textContent

  it.each([
    'deploy-what',
    'deploy-review',
    'deploy-follow-running',
    'deploy-follow-live',
    'deploy-follow-failed',
  ] as const)('%s: the path bar names the deploy, and its environment opens the environment', async (id) => {
    await board(id)
    expect(nav()).toBe('Projects / registry / dev / Deploy')
    await click(btn('dev'))
    expect(el.querySelector('aside[aria-label="Deploy steps"]')).toBeNull()
    expect(selectedTab()).toBe('Overview')
  })

  it('the check step has the same way out', async () => {
    await mount(boards['deploy-what'].status, route({ env: DEV.id, step: 'check' }))
    expect(nav()).toBe('Projects / registry / dev / Deploy')
    await click(btn('registry'))
    expect(el.querySelector('[role=toolbar] nav')?.textContent).toBe('Projects / registry')
  })

  it('step what: Cancel goes back to the environment', async () => {
    const h = await board('deploy-what')
    const before = worker(h.calls).length
    await click(btn('Cancel'))
    expect(selectedTab()).toBe('Overview')
    expect(nav()).toBe('Projects / registry / dev')
    expect(
      worker(h.calls)
        .slice(before)
        .filter((c) => c.id !== 'iii-cloud::api'),
    ).toEqual([])
  })
})

describe('Dep1 source: the branch the check starts from', () => {
  const s = boards['deploy-what'].status
  const p = s.project as NonNullable<Status['project']>
  const ref = (name: string, ahead: number | null, extra: Partial<BranchRef> = {}): BranchRef => ({
    name,
    remote: name.startsWith('origin/'),
    sha: `${name}-sha`,
    committed_at: '2026-11-26T10:00:00Z',
    ahead,
    is_env_branch: false,
    ...extra,
  })
  const withBranches = (mine: number | null, envAhead: number | null = 0): Status => ({
    ...s,
    project: {
      ...p,
      checkout: {
        ...p.checkout,
        branches: [
          ref('origin/old', 1),
          ref('dev', envAhead, { is_env_branch: true }),
          ref('feat/search-rank', mine),
          ref('spike', null),
        ],
      },
    },
  })
  const select = () => el.querySelector('select[aria-label=Source]') as HTMLSelectElement
  const options = () => [...select().options].map((o) => o.textContent)
  const pick = (value: string) =>
    act(async () => {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value')?.set?.call(select(), value)
      select().dispatchEvent(new Event('change', { bubbles: true }))
    })
  const use = (h: { calls: Call[] }) => worker(h.calls).find((c) => c.id === 'iii-cloud::project::use')?.payload

  it('the folder first, then what dev deploys, then the others; the folder is the default when it has commits dev lacks', async () => {
    const h = await mount(withBranches(3), route({ env: DEV.id }))
    expect(text()).not.toContain("You're on")
    expect(options()).toEqual(['feat/search-rank · your folder', 'dev · what dev deploys', 'origin/old', 'spike'])
    expect(select().value).toBe('')
    // The folder's own commits are listed under it.
    expect(el.querySelectorAll(`[aria-label="Commits dev doesn't have"] li`)).toHaveLength(3)
    await click(btn('Check before deploy'))
    expect(use(h)).toEqual({
      path: '/home/you/workspaces/registry',
      environment_id: DEV.id,
      mode: 'worktree',
      from: 'checkout',
      bring_changes: true,
    })
  })

  it('nothing in the folder dev lacks: what dev deploys is the default, and goes as the source', async () => {
    const h = await mount(withBranches(0), route({ env: DEV.id }))
    expect(select().value).toBe('dev')
    expect(el.querySelector(`[aria-label="Commits dev doesn't have"]`)).toBeNull()
    expect(text()).toContain("nothing dev doesn't have")
    expect(text()).toContain('They come from your folder and go on top of dev as one commit you can review.')
    await click(btn('Check before deploy'))
    expect(use(h)).toMatchObject({ from: 'checkout', source: 'dev' })
  })

  it('another branch: only its count under it, and it goes as the source', async () => {
    const h = await mount(withBranches(3), route({ env: DEV.id }))
    await pick('origin/old')
    expect(select().value).toBe('origin/old')
    expect(text()).toContain("1 commit dev doesn't have")
    expect(el.querySelector(`[aria-label="Commits dev doesn't have"]`)).toBeNull()
    await pick('spike')
    // dev's branch isn't on origin for it: no count rather than 0.
    expect(text()).not.toContain("0 commits dev doesn't have")
    await pick('origin/old')
    await click(btn('Check before deploy'))
    expect(use(h)).toMatchObject({ from: 'checkout', source: 'origin/old' })
  })

  it('the note says the check starts from the branch and adds what you pick; production keeps its own row', async () => {
    await mount(withBranches(3), route({ env: DEV.id }))
    expect(text()).toContain(
      'dev deploys whatever is on dev. The check starts from it and adds what you pick here; after the check passes, your deploy pushes the checked commit there, never forced, so nothing on that branch is lost.',
    )
    expect(text()).not.toContain('Your branch has to contain everything on it')
    const prod = { ...withBranches(3).project!, environment: { ...base.project!.environment!, url: null } }
    await mount({ ...s, project: prod }, route({ env: prod.environment.id }, 2))
    expect(el.querySelector('select[aria-label=Source]')).toBeNull()
    expect(text()).toContain("You're on")
  })
})
