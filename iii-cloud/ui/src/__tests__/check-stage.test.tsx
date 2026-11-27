// The check on the Deploy screen (CheckStage): its header says the next thing to do with the lists' own numbers; You
// has the action in each item (a secret's field, a decision's two ways, the check's words for the rest); Your agent
// starts by itself and keeps Stop on screen; Details is one closed fold; Ready says what the deploy does.
import type { Host } from '@iii-dev/console-ui'
import { act } from 'react'
import { describe, expect, it, type Mock, vi } from 'vitest'
import { LaunchNote } from '../AgentRow'
import { CheckStage } from '../CheckStage'
import { boards } from '../fixtures'
import { agent, apiPath, base, pr, ready } from '../fixtures/base'
import { found, passed, rcFixing, rcVariables } from '../fixtures/check'
import { DEV, live6, serving5 } from '../fixtures/deploy'
import { devValues } from '../fixtures/values'
import { hubOf, useStatus } from '../hub'
import type { Route } from '../route'
import type { ChangeEvent, Status } from '../types'
import { fakeHost } from './host'
import { btn, click, cloudAnswers, el, flush, lastCall, root, text, type, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

/** The stage as the area mounts it, on the hub's live status, with the launch note CloudPage shows above it. */
function Live({ narrow = false, ...props }: { host: Host; go: (r: Route) => void; narrow?: boolean }) {
  const { status } = useStatus(props.host, { poll: false })
  return status ? (
    <>
      <LaunchNote />
      <CheckStage {...props} status={status} narrow={narrow} />
    </>
  ) : null
}

type Opts = { openScreen?: Mock; narrow?: boolean }
let mounts = 0
async function stage(
  status: Parameters<typeof fakeHost>[0],
  answers: Parameters<typeof fakeHost>[1] = {},
  opts: Opts = {},
) {
  const h = fakeHost(status, answers)
  const host = h.host as unknown as { panels: Record<string, unknown> }
  if (opts.openScreen) host.panels.openScreen = opts.openScreen
  const go = vi.fn()
  mounts += 1
  await act(async () => root.render(<Live key={mounts} host={h.host} go={go} narrow={opts.narrow} />))
  await flush()
  return { ...h, go }
}

const B = (id: keyof typeof boards) => boards[id].status
const title = () => el.querySelector('h2')?.textContent
const region = (name: string) => el.querySelector(`[role=region][aria-label="${name}"]`)
const fire = (h: ReturnType<typeof fakeHost>, ev: ChangeEvent) =>
  act(async () => h.handlers.get('iii::iii-cloud-ui::events::console-test')?.(ev))
const handoffs = (h: { calls: { id: string }[] }) => h.calls.filter((c) => c.id === 'iii-cloud::handoff')
const marker = { 'iii-cloud::handoff': { id: 'ho-9', marker: '[iii-cloud handoff ho-9]' } }
const deployButton = () =>
  [...el.querySelectorAll('button')].find((b) => /^(Deploy to dev|Deploy \w{7} to \w+)/.test(b.textContent ?? ''))
const youItems = () => [...el.querySelectorAll('li.iii-cloud-cs-need')]
const agentLines = () => [...(region('Your agent')?.querySelectorAll('li.iii-cloud-cs-item') ?? [])]
const countOf = (name: string) => region(name)?.querySelector('.iii-cloud-count')?.textContent
const save = (row: Element | undefined) =>
  [...(row?.querySelectorAll('button') ?? [])].find((b) => b.textContent === 'Save')
const stopPanel = (headline: string) =>
  [...el.querySelectorAll('[role=status]')].find((p) => p.querySelector('b')?.textContent === headline)

describe('opening the screen', () => {
  it('with no check it starts nothing: the header line, what goes, Check; Check starts the check', async () => {
    const h = await stage({ ...B('deploy-checking'), check: null, checks: [] })
    expect(title()).toBe('Check billing for dev')
    expect(text()).toContain('billing: 2 commits, to dev')
    expect(text()).toContain('api/worker-compose-prod.yaml')
    expect(deployButton()?.hasAttribute('disabled')).toBe(true)
    // The canvas draws no line under the header's action; the phone's bar is where it says when Deploy unlocks.
    expect(el.querySelector('.iii-cloud-stephead-reason')).toBeNull()
    expect(text()).not.toContain('Unlocks when')
    expect(h.calls.filter((c) => c.id !== 'iii-cloud::status')).toEqual([])
    await click(btn('Check'))
    expect(lastCall(h.calls)).toEqual({
      id: 'iii-cloud::check',
      payload: { run: true, push: true, manifest: 'api/worker-compose-prod.yaml' },
    })
  })
})

describe('the title is the next thing to do, and its numbers are the lists', () => {
  const statuses: [string, Status][] = [
    ['problems', B('deploy-problems')],
    ['fixing', B('deploy-fixing')],
    ['decision', B('deploy-decision')],
    ['an open problem', B('deploy-open-problem')],
    ['release-control test', rcFixing],
  ]
  it.each(
    statuses,
  )('%s: N things need you · your agent is fixing M, as many as the lists hold', async (_name, status) => {
    await stage(status)
    const t = title() ?? ''
    const you = Number(/(\d+) (?:things?|values?) needs? you/.exec(t)?.[1] ?? 0)
    const agentN = Number(
      /(?:fixing|for your agent to fix)/.test(t)
        ? (/(\d+)(?: for your agent| *$)|fixing (\d+)/.exec(t)?.slice(1).find(Boolean) ?? 0)
        : 0,
    )
    expect(you).toBe(youItems().length)
    expect(you ? countOf('You') : undefined).toBe(you ? String(you) : undefined)
    expect(agentN).toBe(agentLines().length)
    expect(agentN ? countOf('Your agent') : undefined).toBe(agentN ? String(agentN) : undefined)
  })

  it('says only the half that is not zero', async () => {
    await stage(B('deploy-fixing'))
    expect(title()).toBe('Your agent is fixing 1')
    await stage(B('deploy-open-problem'))
    expect(title()).toBe('1 thing needs you · 1 for your agent to fix')
  })

  it('while the check runs, the title is the check; You already lists what is known to be missing', async () => {
    await stage(B('deploy-checking'))
    expect(title()).toBe('Checking billing for dev')
    expect(region('You')?.textContent).toContain('STRIPE_SECRET_KEY')
    expect(region('You')?.textContent).toContain('You can do this while the check runs')
    const parts = region('The check')
    expect(parts?.querySelector('[aria-label="iii cloud\'s plan"]')).toBeTruthy()
    expect(parts?.querySelector('[aria-label="Local run"]')).toBeTruthy()
    expect(deployButton()?.hasAttribute('disabled')).toBe(true)
  })
})

describe('You', () => {
  it('a missing secret is saved from its own row, without leaving the screen, then the check runs again', async () => {
    const h = await stage(B('deploy-problems'), {
      'iii-cloud::values::save': { environment_id: 'e', results: [{ name: 'STRIPE_SECRET_KEY', ok: true }] },
    })
    const row = youItems()[0]
    expect(row.textContent).toContain('STRIPE_SECRET_KEY')
    expect(row.textContent).toContain('has no value in dev')
    expect(row.textContent).toContain('Saved to dev only; your agent never sees it.')
    expect(btn('Open Settings')).toBeNull()
    await type(row.querySelector('input'), 'sk_live_123')
    await click(save(row))
    const saved = h.calls.find((c) => c.id === 'iii-cloud::values::save')
    expect(saved?.payload).toMatchObject({
      items: [{ name: 'STRIPE_SECRET_KEY', from: 'typed', type: 'secret', value: 'sk_live_123' }],
    })
    expect(lastCall(h.calls)).toMatchObject({ id: 'iii-cloud::check' })
    expect(row.textContent).toContain('Saved. The check runs again.')
    // The value reaches only values::save: never the chat or the hand-off.
    const rest = h.calls.filter((c) => c.id !== 'iii-cloud::values::save')
    expect(rest.some((c) => JSON.stringify(c).includes('sk_live_123'))).toBe(false)
    expect(h.go).not.toHaveBeenCalled()
  })

  it('a missing secret can be left empty on purpose: values::save leave_empty, then the check runs again', async () => {
    const h = await stage(B('deploy-problems'), {
      'iii-cloud::values::save': { environment_id: 'e', results: [{ name: 'STRIPE_SECRET_KEY', ok: true }] },
    })
    const row = youItems()[0]
    await click([...row.querySelectorAll('button')].find((b) => b.textContent === 'Leave it empty'))
    expect(h.calls.find((c) => c.id === 'iii-cloud::values::save')?.payload).toMatchObject({
      items: [{ name: 'STRIPE_SECRET_KEY', from: 'leave_empty' }],
    })
    expect(lastCall(h.calls)).toMatchObject({ id: 'iii-cloud::check' })
  })

  it('a value that could not be saved says why and stays', async () => {
    await stage(B('deploy-problems'), {
      'iii-cloud::values::save': {
        environment_id: 'e',
        results: [{ name: 'STRIPE_SECRET_KEY', ok: false, error: { code: 'NOT_SAVED', message: 'x' } }],
      },
    })
    const row = youItems()[0]
    await type(row.querySelector('input'), 'v')
    await click(save(row))
    expect(row.querySelector('input')).toBeTruthy()
    expect(row.querySelector('.iii-cloud-cs-err')?.textContent).toContain("can't write variables")
  })

  it('a decision: the two ways side by side, each with its effect, the recommended one marked; the choice is the call', async () => {
    const h = await stage(B('deploy-decision'))
    const row = youItems().find((r) => r.textContent?.includes("web was changed in iii cloud's console"))
    expect(row).toBeTruthy()
    const opts = [...(row?.querySelectorAll('.iii-cloud-cs-opt') ?? [])]
    expect(opts).toHaveLength(2)
    expect(opts[0].textContent).toContain('Use my compose file')
    expect(opts[0].textContent).toContain("The console's value is removed")
    expect(opts[1].textContent).toContain("Keep the console's value")
    expect(opts[1].textContent).toContain('Your agent changes api/worker-compose-prod.yaml to match the console')
    // The file opens web to the public: the console's value is the recommended one.
    expect(opts[1].hasAttribute('data-best')).toBe(true)
    expect(opts[0].hasAttribute('data-best')).toBe(false)
    expect(opts[1].textContent).toContain('Recommended.')
    expect(opts[0].textContent).toContain('It makes the service public.')
    await click(opts[0])
    expect(lastCall(h.calls)).toEqual({
      id: 'iii-cloud::override::choose',
      payload: { environment_id: expect.any(String), setting: 'web', choice: 'file' },
    })
  })

  it("keeping the console's value hands the edit to the agent, then records the choice", async () => {
    const h = await stage(B('deploy-decision'), { ...marker })
    const row = youItems().find((r) => r.textContent?.includes('was changed in iii cloud'))
    await click(row?.querySelectorAll('.iii-cloud-cs-opt')[1])
    await flush()
    expect(handoffs(h)).toHaveLength(1)
    expect(lastCall(h.calls)).toMatchObject({
      id: 'iii-cloud::override::choose',
      payload: { setting: 'web', choice: 'console' },
    })
  })

  it('any other problem says what, why and the fix: never only a path', async () => {
    await stage(B('deploy-open-problem'))
    const row = youItems()[0]
    expect(row.textContent).toContain('A credential is written in your compose file')
    expect(row.textContent).toContain('anyone who reads the repository can read it')
    expect(row.textContent).toContain('Move the key to a secret and have the file read that secret.')
    expect(row.textContent).toContain('api/worker-compose-prod.yaml:41')
    for (const r of youItems()) expect(r.querySelector('.iii-cloud-cs-label')?.textContent?.length).toBeGreaterThan(10)
  })

  it('the note is true for what is in the card', async () => {
    await stage(B('deploy-problems'))
    expect(region('You')?.textContent).toContain('Secrets go to dev only. Your agent never sees them.')
    await stage(B('deploy-open-problem'))
    expect(region('You')?.textContent).not.toContain('never sees')
    expect(region('You')?.textContent).toContain("Your agent can't do these")
  })

  describe('names a file of the folder can fill: one row, the bulk editor in a dialog', () => {
    const first = () => ({ ...B('deploy-problems'), deploys: [] })
    const read = (items = devValues.items) => ({
      'iii-cloud::values': { ...devValues, environment_id: first().project?.environment?.id, items },
      'iii-cloud::values::save': { environment_id: 'e', results: [{ name: 'SEARCH_WEIGHT_STARS', ok: true }] },
    })
    const fillRow = () =>
      [...el.querySelectorAll('li.iii-cloud-cs-filled')].find((r) => r.textContent?.includes('can be filled'))

    it('shows the numbers and the files, and counts as one item of You and of the title', async () => {
      await stage(first(), read(devValues.items.map((i) => ({ ...i, suggestions: [] }))))
      const n = Number(/^(\d+) (?:thing|value)/.exec(title() ?? '')?.[1])
      await stage(first(), read())
      expect(fillRow()?.textContent).toContain(
        '2 of 4 names your compose file reads can be filled from .env and .env.local.',
      )
      expect(fillRow()?.textContent).toContain('Review and save')
      expect(title()).toMatch(new RegExp(`^${n + 1} (?:things?|values?) `))
    })

    it('its action opens the dialog; saving calls values::save once, then the check', async () => {
      const h = await stage(first(), read())
      expect(el.querySelector('[role=dialog]')).toBeNull()
      await click(btn('Review and save'))
      expect(el.querySelector('[role=dialog]')?.textContent).toContain('Secrets and variables for')
      await click(
        [...el.querySelectorAll('[role=dialog] button')].find((b) =>
          /^Save \d+ values? and check again/.test(b.textContent ?? ''),
        ),
      )
      const saves = h.calls.filter((c) => c.id === 'iii-cloud::values::save')
      expect(saves).toHaveLength(1)
      expect(saves[0].payload).toMatchObject({
        items: expect.arrayContaining([
          { name: 'SEARCH_WEIGHT_STARS', from: 'env_file', type: 'variable', file: '.env' },
        ]),
      })
      const ids = h.calls.map((c) => c.id)
      expect(ids.lastIndexOf('iii-cloud::check')).toBeGreaterThan(ids.indexOf('iii-cloud::values::save'))
    })

    it('with nothing fillable the row is absent, and a name no file fills keeps its own row', async () => {
      await stage(first(), read(devValues.items.map((i) => ({ ...i, suggestions: [] }))))
      expect(fillRow()).toBeUndefined()
      expect(youItems().some((r) => r.textContent?.includes('STRIPE_SECRET_KEY'))).toBe(true)
    })
  })

  it('the secrets page is one link away, for what the row cannot do', async () => {
    const h = await stage(B('deploy-problems'))
    await click(btn('Secrets and variables'))
    expect(h.go).toHaveBeenCalledWith({ at: 'secrets', env: expect.any(String) })
  })

  it('the first time: the names filled from .env files, with Review', async () => {
    const dev = B('deploy-problems')
    const h = await stage(
      { ...dev, deploys: [] },
      {
        'iii-cloud::values': {
          environment_id: dev.project?.environment?.id,
          environment: 'dev',
          manifest: 'api/worker-compose-prod.yaml',
          read_from: 'workspace',
          cloud: 'read',
          save: 'ok',
          items: [
            ...Array.from({ length: 4 }, (_, i) => ({
              name: `FILLED_${i}`,
              need: 'required',
              in_cloud: 'set',
              here: 'from_file',
              here_file: 'api/.env.test',
              suggestions: [],
            })),
            { name: 'OPTIONAL_ONE', need: 'optional', in_cloud: 'unset', suggestions: [] },
          ],
        },
      },
    )
    expect(region('You')?.textContent).toContain(
      '4 of 5 names your compose file reads were filled from .env.test, saved as secrets. 1 optional one is empty.',
    )
    await click(btn('Review'))
    expect(h.go).toHaveBeenCalledWith({ at: 'secrets', env: expect.any(String) })
  })
})

describe('Your agent', () => {
  const ended = (record: object) =>
    ({ kind: 'check', id: 'chk-d1', terminal: true, summary: '', record }) as ChangeEvent

  it('starts without a click when auto_fix is on: once, when the check it watched ends with problems for it', async () => {
    const h = await stage(B('deploy-checking'), marker)
    expect(btn('Fix with your agent')).toBeNull()
    await fire(h, ended(found))
    await flush()
    expect(handoffs(h)).toHaveLength(1)
    expect(handoffs(h)[0]).toMatchObject({ payload: { kind: 'ready', check_id: 'chk-d1' } })
    await fire(h, ended({ ...found }))
    await flush()
    expect(handoffs(h)).toHaveLength(1)
  })

  it('starts the same with auto_fix true or absent, and not when it is false', async () => {
    const on = await stage({ ...B('deploy-checking'), auto_fix: true }, marker)
    await fire(on, ended(found))
    await flush()
    expect(handoffs(on)).toHaveLength(1)
    const off = await stage({ ...B('deploy-checking'), auto_fix: false }, marker)
    await fire(off, ended(found))
    await flush()
    expect(handoffs(off)).toHaveLength(0)
    // It does not go quiet: the card offers the control the sentence points at.
    expect(region('Your agent')).toBeTruthy()
    await click(btn('Fix with your agent'))
    expect(handoffs(off)).toHaveLength(1)
  })

  it('never with an agent active, nor for a check the agent asked for; there is no checkbox', async () => {
    const busy = await stage(
      { ...B('deploy-checking'), agent: { ...B('deploy-fixing').agent!, kind: 'import' } },
      marker,
    )
    await fire(busy, ended(found))
    await flush()
    expect(handoffs(busy)).toHaveLength(0)
    const asked = await stage(B('deploy-checking'), marker)
    await fire(asked, ended({ ...found, handoff_id: 'ho-x' }))
    await flush()
    expect(handoffs(asked)).toHaveLength(0)
    expect(el.querySelector('input[type=checkbox]')).toBeNull()
    expect(text()).not.toContain('Start my agent')
  })

  it('working: one line each with the current one marked, how long and where, Open chat, a visible Stop', async () => {
    const openScreen = vi.fn()
    const h = await stage(B('deploy-fixing'), { 'iii-cloud::handoff::stop': {} }, { openScreen })
    expect(region('Your agent')?.textContent).toMatch(/working 4 min, in a separate copy/)
    const lines = agentLines()
    expect(lines).toHaveLength(1)
    expect(lines[0].querySelector('[data-mark=work]')).toBeTruthy()
    expect(lines[0].textContent).toContain('now · Removing pg_stat_statements from 0008_trgm.sql')
    expect(btn('Stop')).toBeTruthy()
    await click(btn('Open chat'))
    expect(openScreen).toHaveBeenCalledWith({ screen: 'chat:sess-1' })
    await click(btn('Stop'))
    expect(el.querySelector('[role=alertdialog]')?.textContent).toContain(
      'Its commits so far stay on iii-cloud/ready-dev.',
    )
    await click([...el.querySelectorAll('[role=alertdialog] button')].find((b) => b.textContent === 'Stop the agent'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::handoff::stop', payload: { id: 'ho-d' } })
  })

  it('a waiting agent says what it asks, with the chat to answer in', async () => {
    const waiting = {
      ...B('deploy-fixing'),
      agent: { ...agent('waiting_for_you'), handoff_id: 'ho-d', kind: 'ready' as const },
    }
    await stage(waiting, {}, { openScreen: vi.fn() })
    expect(region('Your agent')?.textContent).toContain('Keep path:// for http and storage')
    expect(btn('Open chat')).toBeTruthy()
  })
})

describe('Details', () => {
  it('one fold, closed by default: Done, Warnings and Checks; a skipped name is under what it is', async () => {
    await stage({ ...B('deploy-fixing'), skipped_values: ['SENTRY_DSN'] })
    const fold = region('Details')
    expect(fold?.querySelector('button')?.getAttribute('aria-expanded')).toBe('false')
    expect(fold?.querySelector('button')?.textContent).toMatch(/Done \d+ · Warnings \d+ · Checks \d+/)
    expect(el.querySelector('[role=region][aria-label="Skipped"]')).toBeNull()
    await click(fold?.querySelector('button'))
    expect(fold?.querySelector('button')?.getAttribute('aria-expanded')).toBe('true')
    expect(fold?.querySelector('[aria-label="Warnings"]')?.textContent).toContain('SENTRY_DSN has no value')
    expect(fold?.querySelector('[aria-label="Checks"]')).toBeTruthy()
  })
})

describe('Ready', () => {
  const dev = B('deploy-passed')

  it('Ready to deploy to dev; the rows say what this deploy does; the button deploys', async () => {
    const h = await stage(dev, { 'iii-cloud::deploy': { id: 'dep-9', number: 9 } })
    expect(title()).toBe('Ready to deploy to dev')
    expect(text()).toContain("passed iii cloud's plan and the local run at")
    const card = region('What this deploy does')
    expect(card?.textContent).toContain('Code')
    expect(card?.textContent).toContain('Compose file')
    expect(card?.textContent).toContain('api/worker-compose-prod.yaml')
    expect(card?.textContent).toContain('9 workers in 3 groups')
    // The release command is the one the local run ran; a database row only when the plan creates one.
    expect(card?.textContent).toContain('Release commanddb:migrate')
    expect(card?.textContent).not.toContain('Database')
    expect(text()).toContain("Only you can confirm this; your agent can't.")
    expect(el.querySelector('input[type=checkbox]')).toBeNull()
    await click(btn('Deploy 9c04e7f to dev'))
    expect(h.calls.find((c) => c.id === 'iii-cloud::deploy')?.payload).toEqual({ check_id: 'chk-d4' })
    expect(h.go).toHaveBeenCalledWith({ at: 'deploy', env: expect.any(String), deploy: 'dep-9' })
  })

  it("an assisted deploy says the agent's fixes land on the environment's branch and the person's branch is not changed", async () => {
    // Newest first: two commits by the agent, then the last one the folder gave (the workspace's from_head).
    const commits = [
      { sha: 'a'.repeat(40), message: 'Pin search' },
      { sha: 'b'.repeat(40), message: 'Fix it' },
      { sha: 'e41a7c3'.padEnd(40, '0'), message: 'Search worker' },
    ]
    await stage({ ...dev, check: { ...passed, commits } })
    // `dev` is not production: an assisted deploy puts the fixes on the environment's own branch, never on billing.
    expect(region('What this deploy does')?.textContent).toContain(
      "After the deploy, dev has your agent's 2 fixes, and billing is not changed",
    )
  })

  // An environment that is not production, on the default branch, with no pull request anywhere.
  const onDefault = (brief?: boolean): Status => {
    const p = dev.project!
    const env = { ...p.environment!, name: 'test', production: false, branch: 'main' }
    return {
      ...dev,
      pr: null,
      project: {
        ...p,
        default_branch: 'main',
        environment: env,
        environments: [{ ...env }, { ...env, id: 'other-prod', name: 'prod', production: true }],
      },
      ...(brief === undefined
        ? {}
        : {
            handoff_brief: {
              ...(dev.handoff_brief ?? ({} as NonNullable<Status['handoff_brief']>)),
              pushes_directly: brief,
            },
          }),
    }
  }

  it('an assisted deploy of an environment on the default branch: no pull request, its own branch, Deploy enabled', async () => {
    // Whatever the hand-off's brief says: the page decides from production alone.
    for (const brief of [undefined, false, true]) {
      const h = await stage(onDefault(brief), { 'iii-cloud::deploy': { id: 'dep-9', number: 9 } })
      const card = region('What this deploy does')?.textContent ?? ''
      expect(card, String(brief)).toContain('How it shipsAssisted deploy')
      expect(card).toContain('This page sends 9c04e7f to iii-cloud/test, and iii cloud deploys it.')
      expect(card).toContain('Branchtest moves from main to its own branch, iii-cloud/testmain is not pushed.')
      expect(text()).not.toContain('Ask your agent to open it')
      expect(btn('Deploy 9c04e7f to test')?.hasAttribute('disabled')).toBe(false)
      await click(btn('Deploy 9c04e7f to test'))
      expect(h.calls.find((c) => c.id === 'iii-cloud::deploy')?.payload).toEqual({ check_id: dev.check!.id })
    }
  })

  it('the Branch row warns about merges only when something serves the environment', async () => {
    await stage(onDefault(), cloudAnswers({}))
    expect(region('What this deploy does')?.textContent).not.toContain('no longer deploys')
    const s = onDefault()
    const env = s.project!.environment!.id
    const serving = {
      id: serving5.id,
      number: serving5.number,
      environment_id: env,
      type: 'push',
      status: 'ready',
      current: true,
      commit: { sha: serving5.commit.sha },
      triggered_by: { user_id: null, api_key_id: null },
      created_at: serving5.created_at,
      finished_at: null,
    }
    await stage(
      { ...s, deploys: [serving5] },
      cloudAnswers({ [apiPath('/deployments', env)]: { deployments: [serving] } }),
    )
    expect(region('What this deploy does')?.textContent).toContain(
      'After this deploy, a merge into main no longer deploys test.',
    )
  })

  it('a local run that was reused is still a pass', async () => {
    const reused = { ...passed.run, reused: true } as NonNullable<typeof passed.run>
    await stage({ ...dev, check: { ...passed, run: reused } })
    expect(title()).toBe('Ready to deploy to dev')
    expect(btn('Deploy 9c04e7f to dev')?.hasAttribute('disabled')).toBe(false)
  })

  it('a deployment of dev still going locks the button and says so, with Follow', async () => {
    const going = boards['deploy-follow-running'].status.deploys[0]
    const h = await stage({
      ...dev,
      deploys: [{ ...going, check_id: 'other', environment_id: dev.project?.environment?.id ?? '' }],
    })
    expect(btn('Deploy 9c04e7f to dev')?.hasAttribute('disabled')).toBe(true)
    expect(text()).toContain('Deployment #6 to dev is already running')
    await click(btn('Follow #6'))
    expect(h.go).toHaveBeenCalledWith({ at: 'deploy', env: expect.any(String), deploy: 'dep-6' })
  })

  it('production asks for its name before it deploys', async () => {
    const h = await stage(
      { ...base, check: ready, checks: [], pr },
      { 'iii-cloud::deploy': { id: 'dep-1', number: 1 } },
    )
    await click(btn('Merge to main and deploy to production…'))
    expect(h.calls.some((c) => c.id === 'iii-cloud::deploy')).toBe(false)
    await type(el.querySelector('[role=dialog] input'), 'production')
    await click([...el.querySelectorAll('[role=dialog] button')].find((b) => b.textContent === 'Merge and deploy'))
    expect(h.calls.find((c) => c.id === 'iii-cloud::deploy')?.payload).toEqual({ check_id: ready.id, pr: pr.number })
  })
})

describe('when the check cannot finish', () => {
  it('the local run cannot start (no Docker): the plan still shows, with Run again', async () => {
    const h = await stage(B('state-docker'))
    expect(stopPanel("Docker isn't running on this machine")).toBeTruthy()
    expect(text()).toContain('The local run starts a database and your workers in containers.')
    await click(btn('Run again'))
    expect(lastCall(h.calls)).toMatchObject({ id: 'iii-cloud::check' })
  })

  it('out of date: the source moved after the check; the deploy stays locked', async () => {
    await stage(B('state-outdated'))
    expect(stopPanel('billing moved after the check')).toBeTruthy()
    expect(btn('Deploy 9c04e7f to dev')?.hasAttribute('disabled')).toBe(true)
  })

  it('a quiet agent: no progress, with the chat and Stop', async () => {
    await stage(B('state-stalled'), {}, { openScreen: vi.fn() })
    expect(region('Your agent')?.textContent).toContain('No progress for 10 min')
    expect(btn('Stop')).toBeTruthy()
  })

  it('a stopped agent: what it could not fix, ask it to finish or check again', async () => {
    await stage(B('state-stopped'))
    expect(stopPanel("Your agent couldn't fix 1 problem")).toBeTruthy()
    expect(btn('Ask your agent to finish')).toBeTruthy()
    expect(btn('Check again')).toBeTruthy()
  })

  it('another deployment running: Follow it', async () => {
    const h = await stage(B('state-busy'))
    expect(text()).toContain('Deployment #6 to dev is already running')
    await click(btn('Follow #6'))
    expect(h.go).toHaveBeenCalledWith({ at: 'deploy', env: expect.any(String), deploy: 'dep-dev-6' })
  })

  it('signed out: sign in again, nothing else is lost', async () => {
    const h = await stage(B('state-signedout'))
    expect(stopPanel('Your iii cloud sign-in expired')).toBeTruthy()
    await click(btn('Sign in again'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::connect', payload: { method: 'browser' } })
  })

  it('a replaced run is not an error', async () => {
    const run = { ...found.run, status: 'failed', error: { stage: 'build', code: 'RUN_REPLACED', message: 'x' } }
    await stage({ ...B('deploy-problems'), check: { ...found, run: run as NonNullable<typeof found.run> } })
    expect(el.querySelector('.iii-cloud-cs-stop')).toBeNull()
  })

  it('a worker that sends no reused, auto_fix or fix still works', async () => {
    await stage({ ...B('deploy-fixing'), auto_fix: undefined })
    expect(title()).toBe('Your agent is fixing 1')
  })
})

describe('a narrow pane', () => {
  it('one column; the primary action is in a bar at the bottom, not in the header', async () => {
    await stage(B('deploy-passed'), {}, { narrow: true })
    expect(el.querySelector('.iii-cloud-cs[data-narrow]')).toBeTruthy()
    expect(el.querySelector('.iii-cloud-cs-bar')?.textContent).toContain('Deploy 9c04e7f to dev')
    expect(el.querySelector('.iii-cloud-stephead-fwd')).toBeNull()
  })
})

describe('the review: what a check that is not ready still offers', () => {
  const noAgent = (s: Status) => ({ ...s, agent: null, handoff_brief: null })
  const deployButtons = () => [...el.querySelectorAll('button')].filter((b) => /^Deploy\b/.test(b.textContent ?? ''))

  it('Check again is on screen when nothing is in flight: primary with nothing else to do, beside Deploy otherwise', async () => {
    const quiet = await stage(noAgent({ ...B('deploy-passed'), check: { ...passed, ready: false } }))
    expect(btn('Check again')?.getAttribute('data-variant')).toBe('primary')
    await click(btn('Check again'))
    expect(lastCall(quiet.calls)).toMatchObject({ id: 'iii-cloud::check' })
    await stage(B('deploy-problems'))
    expect(btn('Check again')?.getAttribute('data-variant')).toBe('secondary')
    expect(deployButtons().every((b) => b.hasAttribute('disabled'))).toBe(true)
  })

  it('"Deploy without it" obeys every lock of the primary deploy: another deployment, signed out, a deploy being sent', async () => {
    const outdated = B('state-outdated')
    const without = () => deployButtons().find((b) => b.textContent?.endsWith('without it'))
    await stage(outdated)
    expect(without()?.hasAttribute('disabled')).toBe(false)
    await stage({ ...outdated, deploys: B('state-busy').deploys })
    expect(without()?.hasAttribute('disabled')).toBe(true)
    await stage({ ...outdated, connection: { ...outdated.connection, status: 'expired' } })
    expect(without()?.hasAttribute('disabled')).toBe(true)
    const sending = await stage(outdated, { 'iii-cloud::deploy': () => new Promise(() => undefined) })
    await click(without())
    expect(sending.calls.some((c) => c.id === 'iii-cloud::deploy')).toBe(true)
    expect(deployButtons().every((b) => b.hasAttribute('disabled'))).toBe(true)
  })

  it('an agent the person stopped is not started again when the check ends; "Fix with your agent" is', async () => {
    // A check runs and the agent works; the person presses Stop (as on screen), the agent ends, the check ends.
    const working = B('deploy-fixing')
    const h = await stage(working, { ...marker, 'iii-cloud::handoff::stop': {} })
    await act(async () => hubOf(h.host).checking(1))
    await click(btn('Stop'))
    await click([...el.querySelectorAll('[role=alertdialog] button')].find((b) => b.textContent === 'Stop the agent'))
    await act(async () => hubOf(h.host).checking(-1))
    await fire(h, {
      kind: 'agent',
      id: 'ho-d',
      terminal: true,
      summary: '',
      record: { ...working.agent, state: 'stopped' },
    } as ChangeEvent)
    const ended = { ...found, id: 'chk-end', checked_at: new Date(Date.now() + 60_000).toISOString() }
    await fire(h, { kind: 'check', id: 'chk-end', terminal: true, summary: '', record: ended } as ChangeEvent)
    await flush()
    expect(handoffs(h)).toHaveLength(0)
    // The way back is on screen: the stopped agent's notice offers it.
    await click(btn('Ask your agent to finish'))
    expect(handoffs(h)).toHaveLength(1)
  })

  it('the automatic hand-off does not start on the check of a source the person replaced', async () => {
    const running = {
      ...found,
      id: 'chk-run',
      run: { ...found.run!, status: 'running' as const },
      handoff_id: null,
    }
    const h = await stage({ ...B('deploy-checking'), check: running }, marker)
    const env = running.environment_id
    // Change → use (the source is replaced while the check runs) → the new check is refused → the old one ends.
    await act(async () => hubOf(h.host).used(env, true))
    expect(hubOf(h.host).movedFrom(env)).toBe('chk-run')
    const ended = { ...found, id: 'chk-run', checked_at: new Date(Date.now() + 60_000).toISOString(), handoff_id: null }
    await fire(h, { kind: 'check', id: 'chk-run', terminal: true, summary: '', record: ended } as ChangeEvent)
    await flush()
    expect(handoffs(h)).toHaveLength(0)
  })

  it("on a replaced source none of the old check's notices offer work, and Check again checks the picked file", async () => {
    const stopped = B('state-stopped')
    const h = await stage(stopped, { 'iii-cloud::check': {} })
    expect(btn('Ask your agent to finish')).toBeTruthy()
    const env = stopped.check!.environment_id
    await act(async () => {
      hubOf(h.host).used(env, true)
      hubOf(h.host).setPickedManifest(env, 'api/worker-compose.staging.yaml')
    })
    expect(btn('Ask your agent to finish')).toBeNull()
    expect(btn('Bring it into the check')).toBeNull()
    expect(btn('Read what it tried')).toBeNull()
    // The retry checks what was picked with the change, not the old check's file.
    await click(btn('Check again'))
    expect(lastCall(h.calls)).toMatchObject({
      id: 'iii-cloud::check',
      payload: { run: true, manifest: 'api/worker-compose.staging.yaml' },
    })
  })

  const runningOf = (s: Status, over: object) => ({
    ...(boards['deploy-review'].status.deploys.at(0) ?? serving5),
    id: 'dep-run',
    number: 9,
    environment_id: s.check!.environment_id,
    status: 'in_progress' as const,
    outcome: 'running' as const,
    finished_at: null,
    ...over,
  })

  it('a deployment going of the SAME check still stops Deploy: its notice follows it, whatever check it came from', async () => {
    const s = B('deploy-passed')
    await stage({
      ...s,
      deploys: [runningOf(s, { check_id: s.check!.id, created_at: new Date().toISOString() }), ...s.deploys],
    })
    expect(btn('Follow #9')).toBeTruthy()
    expect(deployButtons().every((b) => b.hasAttribute('disabled'))).toBe(true)
  })

  it('a deployment going that is not the newest also stops Deploy: the shared lock counts every running one', async () => {
    const s = B('deploy-passed')
    const newest = {
      ...serving5,
      id: 'dep-newest',
      number: 10,
      environment_id: s.check!.environment_id,
      // Before the check ran: the "environment changed after the check" notice does not fire; only the shared lock does.
      created_at: '2020-01-01T00:00:00.000Z',
    }
    await stage({ ...s, deploys: [newest, runningOf(s, { check_id: null }), ...s.deploys] })
    expect(btn('Follow #9')).toBeNull()
    expect(deployButtons().every((b) => b.hasAttribute('disabled'))).toBe(true)
    expect(text()).toContain('Deployment #9 is still going.')
  })

  it('a deployment held in iii cloud that this machine did not start does not lock Deploy, and says deploying replaces it', async () => {
    const s = B('deploy-passed')
    const held = runningOf(s, {
      id: 'dep-held',
      number: 11,
      status: 'needs_action' as const,
      type: 'push',
      check_id: null,
      approved_by_worker: false,
      from_console: true,
      created_at: new Date(Date.now() + 1000).toISOString(),
    })
    await stage({ ...s, deploys: [held, ...s.deploys] })
    expect(text()).toContain('Deployment #11 is waiting for confirmation in iii cloud')
    expect(text()).toContain('replaces it')
    const primary = deployButtons().find((b) => /^Deploy 9c04e7f/.test(b.textContent ?? ''))
    expect(primary?.hasAttribute('disabled')).toBe(false)
  })

  it("a held deployment is this machine's only by the worker's markers (check_id, approved, created); a pr or the check's commit alone is elsewhere", async () => {
    const s = B('deploy-passed')
    const base = {
      status: 'needs_action' as const,
      type: 'push',
      check_id: null,
      approved_by_worker: false,
      from_console: true,
      created_at: new Date(Date.now() + 1000).toISOString(),
    }
    const ours: Record<string, object> = {
      'its merge (pr and check_id)': { ...base, pr: 5, check_id: s.check!.id },
      'the worker approved it': { ...base, approved_by_worker: true },
      'the worker created it': { ...base, from_console: false },
    }
    // Ours: it counts as going. The screen follows it by its number, and Deploy waits for it.
    for (const [why, over] of Object.entries(ours)) {
      await stage({ ...s, deploys: [runningOf(s, { id: 'dep-held', number: 11, ...over }), ...s.deploys] })
      expect(btn('Follow #11'), why).toBeTruthy()
      expect(text(), why).not.toContain('is waiting for confirmation in iii cloud')
      expect(
        deployButtons().every((b) => b.hasAttribute('disabled')),
        why,
      ).toBe(true)
    }
    // Elsewhere: a merge's `pr` with no worker marker, or a push whose commit is the shown check's (a push by hand,
    // which the worker would refuse to confirm as MAIN_MOVED): nothing to follow, Deploy is open, it says it replaces it.
    const elsewhere: Record<string, object> = {
      'a pr with no worker marker': { ...base, pr: 5 },
      "the shown check's commit (a push by hand)": { ...base, commit: { sha: s.check!.commit, message: 'm' } },
    }
    for (const [why, over] of Object.entries(elsewhere)) {
      await stage({ ...s, deploys: [runningOf(s, { id: 'dep-held', number: 11, ...over }), ...s.deploys] })
      expect(text(), why).toContain('Deployment #11 is waiting for confirmation in iii cloud')
      expect(btn('Follow #11'), why).toBeNull()
      const primary = deployButtons().find((b) => /^Deploy 9c04e7f/.test(b.textContent ?? ''))
      expect(primary?.hasAttribute('disabled'), why).toBe(false)
    }
  })

  it('the production name dialog locks when the check it was opened for stops being the ready one (another pane changed the source)', async () => {
    const s = B('deploy-passed')
    const prod = { ...s.project!, environment: { ...s.project!.environment!, production: true } }
    // Production deploys by pull request: the check's own, open and not behind.
    const open = { ...pr, state: 'OPEN' as const, head_sha: s.check!.commit, main_moved: false }
    const h = await stage({ ...s, project: prod, pr: open }, { 'iii-cloud::deploy': {} })
    await click(btn('Merge to main and deploy to dev…'))
    const dialog = () => el.querySelector('[role=dialog]')
    await type(dialog()?.querySelector('input') ?? null, 'dev')
    const go = () => [...(dialog()?.querySelectorAll('button') ?? [])].find((b) => b.textContent === 'Merge and deploy')
    expect(go()?.hasAttribute('disabled')).toBe(false)
    await act(async () => hubOf(h.host).used(s.check!.environment_id, true))
    expect(go()?.hasAttribute('disabled')).toBe(true)
    expect(dialog()?.textContent).toContain('no longer the check that was ready')
  })

  it('the production name dialog locks when a newer READY check arrives while it is open, with no other lock, and says to close it', async () => {
    const s = B('deploy-passed')
    const prod = { ...s.project!, environment: { ...s.project!.environment!, production: true } }
    // Production deploys by pull request: the check's own, open and not behind.
    const open = { ...pr, state: 'OPEN' as const, head_sha: s.check!.commit, main_moved: false }
    const h = await stage({ ...s, project: prod, pr: open }, { 'iii-cloud::deploy': {} })
    await click(btn('Merge to main and deploy to dev…'))
    const dialog = () => el.querySelector('[role=dialog]')
    await type(dialog()?.querySelector('input') ?? null, 'dev')
    const go = () => [...(dialog()?.querySelectorAll('button') ?? [])].find((b) => b.textContent === 'Merge and deploy')
    expect(go()?.hasAttribute('disabled')).toBe(false)
    await fire(h, {
      kind: 'check',
      id: 'chk-newer-ready',
      terminal: true,
      summary: '',
      record: { ...s.check!, id: 'chk-newer-ready', checked_at: new Date(Date.now() + 60_000).toISOString() },
    } as ChangeEvent)
    expect(go()?.hasAttribute('disabled')).toBe(true)
    expect(dialog()?.textContent).toContain(
      'The ready check changed while this was open. Close it and deploy the new one.',
    )
  })

  it('on a replaced source Details › Warnings offers no action of the old check', async () => {
    const s = B('setup-fixing-live')
    const h = await stage(s)
    const actions = () =>
      [...el.querySelectorAll('.iii-cloud-cs-item button')].filter((b) =>
        /Skip for now|Undo|^Copy /.test(b.textContent ?? ''),
      )
    expect(actions().length).toBeGreaterThan(0)
    await act(async () => hubOf(h.host).used(s.check!.environment_id, true))
    expect(actions()).toHaveLength(0)
  })

  it('the header of a replaced check names the new source and the picked file, with no old commit count', async () => {
    const s = B('deploy-passed')
    const h = await stage(s)
    expect(text()).toContain('passed iii cloud')
    await act(async () => {
      hubOf(h.host).used(s.check!.environment_id, true)
      hubOf(h.host).setPickedManifest(s.check!.environment_id, 'api/worker-compose.staging.yaml')
    })
    const sub = el.querySelector('.iii-cloud-stephead-sub, .iii-cloud-cs-sub')?.textContent ?? ''
    expect(sub).toContain('api/worker-compose.staging.yaml')
    expect(sub).not.toMatch(/\d+ commits?/)
  })

  it("a stalled agent says why when the worker did: its question is the card's text and the environment card's", async () => {
    const why = 'The cloud refused the plan three checks in a row: ENV_FILE_MISSING.'
    const stalled = B('state-stalled')
    await stage({ ...stalled, agent: { ...stalled.agent!, question: why } })
    expect(region('Your agent')?.textContent).toContain(why)
    expect(region('Your agent')?.textContent).not.toContain('No progress for')
    await stage(stalled)
    expect(region('Your agent')?.textContent).toContain('No progress for')
  })

  it("the Ready footer says nothing is live when iii cloud's list only has failed rows, and a deployment it does not show when it may hide one", async () => {
    const s = { ...boards['deploy-review'].status, deploys: [] }
    const row = (status: string) => ({
      id: 'x1',
      number: 1,
      environment_id: DEV.id,
      type: 'push',
      status,
      current: false,
      commit: { sha: 'a'.repeat(40) },
      triggered_by: { user_id: null, api_key_id: null },
      created_at: new Date().toISOString(),
      finished_at: null,
    })
    const list = (deployments: object[], next_cursor: string | null = null) =>
      cloudAnswers({ [apiPath('/deployments', DEV.id)]: { deployments, next_cursor } })
    await stage(s, list([row('failed')]))
    expect(text()).toContain('Nothing is live on dev yet.')
    // A replaced row never served (iii cloud replaces only a deployment that was waiting for confirmation).
    await stage(s, list([row('canceled'), row('replaced')]))
    expect(text()).toContain('Nothing is live on dev yet.')
    await stage(s, list([row('failed'), row('ready')]))
    expect(text()).toContain("serves a deployment iii cloud's list does not show")
    await stage(s, list([row('failed')], 'next-page'))
    expect(text()).toContain("serves a deployment iii cloud's list does not show")
  })

  it('SOURCE_CHANGED (the worker refuses a check whose source was replaced) is shown with Check again, like MAIN_MOVED', async () => {
    const h = await stage(B('deploy-passed'), {
      'iii-cloud::deploy': () => Promise.reject(Object.assign(new Error('x'), { code: 'SOURCE_CHANGED' })),
    })
    await click(deployButtons().find((b) => /^Deploy 9c04e7f/.test(b.textContent ?? '')))
    await flush()
    expect(text()).toContain('The source or the compose file changed after the check.')
    expect(btn('Check again')).toBeTruthy()
    expect(deployButtons().every((b) => b.hasAttribute('disabled'))).toBe(true)
    await click(btn('Check again'))
    expect(h.calls.some((c) => c.id === 'iii-cloud::check')).toBe(true)
  })

  it('the primary Deploy reads the shared lock: it goes off when the worker stops answering', async () => {
    const s = B('deploy-passed')
    let down = false
    const h = await stage(() => {
      if (down) throw new Error('x')
      return s
    })
    const primary = () => [...el.querySelectorAll('button')].find((b) => /^Deploy 9c04e7f/.test(b.textContent ?? ''))
    expect(primary()?.hasAttribute('disabled')).toBe(false)
    down = true
    await act(async () => hubOf(h.host).refresh(true))
    expect(primary()?.hasAttribute('disabled')).toBe(true)
    expect(text()).toContain('out of date')
  })

  it('a source replaced by "Check <tip>" holds Deploy until another check exists, and folds the old check', async () => {
    const outdated = B('state-outdated')
    const h = await stage(outdated, {
      'iii-cloud::project::use': {},
      'iii-cloud::check': () => Promise.reject(Object.assign(new Error('x'), { code: 'NO_COMMITS' })),
    })
    await click([...el.querySelectorAll('button')].find((b) => /^Check [0-9a-f]{7}$/.test(b.textContent ?? '')))
    await flush()
    expect(title()).toMatch(/^Check /)
    expect(
      [...el.querySelectorAll('button')]
        .filter((b) => /^Deploy\b/.test(b.textContent ?? ''))
        .every((b) => b.hasAttribute('disabled')),
    ).toBe(true)
    expect(region('You')).toBeNull()
    expect(region('Details')?.textContent).toContain('before you changed the source')
    // Any other check id is newer, whatever its clock says.
    await fire(h, {
      kind: 'check',
      id: 'chk-new',
      terminal: true,
      summary: '',
      record: { ...outdated.check!, id: 'chk-new', checked_at: new Date(Date.now() + 60_000).toISOString() },
    } as ChangeEvent)
    expect(title()).toBe('Ready to deploy to dev')
  })

  it("the Ready footer names the deployment iii cloud says serves, not the worker's newest; none before the list is read", async () => {
    const s = { ...boards['deploy-review'].status, deploys: [live6, serving5] }
    const env = DEV.id
    const list = (id: string) => ({
      [apiPath('/deployments', env)]: {
        deployments: s.deploys.map((d) => ({
          id: d.id,
          number: d.number,
          environment_id: env,
          type: 'push',
          status: 'ready',
          current: d.id === id,
          commit: { sha: d.commit.sha },
          triggered_by: { user_id: null, api_key_id: null },
          created_at: d.created_at,
          finished_at: null,
        })),
      },
    })
    // The worker's newest ready deployment is #6; iii cloud says #5 serves (a rollback made in the console).
    const old = serving5
    await stage(s, cloudAnswers(list(old.id)))
    expect(text()).toContain(`${s.project?.environment?.name} serves #${old.number} until the new deployment is live.`)
    await stage(s, cloudAnswers({}))
    expect(text()).not.toMatch(/serves #\d+ until/)
    expect(text()).not.toContain('Nothing is live on')
  })

  it('a session that cannot write says who can, with no field: an API key, a Developer on production', async () => {
    const base = B('deploy-problems')
    const key = { secret: 'iii_x', team_id: 't', environment_id: 'e' }
    await stage({ ...base, connection: { ...base.connection, api_key: key, method: 'api_key' } })
    expect(youItems()[0].querySelector('input')).toBeNull()
    expect(youItems()[0].textContent).toContain("An API key can't write secrets and variables.")
    const prod = { ...base.project!, environment: { ...base.project!.environment!, production: true } }
    const teams = base.connection.teams.map((t) => ({ ...t, role: 'Developer' }))
    await stage({ ...base, project: prod, connection: { ...base.connection, teams } })
    expect(youItems()[0].querySelector('input')).toBeNull()
    expect(youItems()[0].textContent).toContain('takes Owner or Admin. You are Developer.')
  })

  it('an optional empty name (the worker sends it as the EMPTY_DEFAULT_VARIABLE warning) is skipped from Warnings; a required secret never offers Skip', async () => {
    const h = await stage(B('setup-fixing-live'), { 'iii-cloud::values::skip': {} })
    const skip = [...el.querySelectorAll('button')].find((b) => b.textContent === 'Skip for now')
    expect(skip?.closest('details, [aria-label=Warnings], .iii-cloud-cs-item')).toBeTruthy()
    await click(skip)
    expect(h.calls.find((c) => c.id === 'iii-cloud::values::skip')?.payload).toMatchObject({
      names: [expect.any(String)],
      skip: true,
    })
    expect(lastCall(h.calls)).toMatchObject({ id: 'iii-cloud::check' })
    // A required secret (SECRET_MISSING blocks) is in You with no Skip, whatever the values read says of its name
    // (a real read, first deploy, `need` required or even optional: the old rule offered Skip on the latter).
    for (const need of ['required', 'optional'] as const) {
      const first = { ...B('deploy-problems'), deploys: [] }
      await stage(first, {
        'iii-cloud::values': {
          environment_id: first.project?.environment?.id,
          environment: 'dev',
          manifest: 'api/worker-compose-prod.yaml',
          read_from: 'workspace',
          cloud: 'read',
          save: 'ok',
          items: [{ name: 'STRIPE_SECRET_KEY', need, in_cloud: 'unset', suggestions: [] }],
        },
      })
      expect(youItems().length).toBeGreaterThan(0)
      expect(
        youItems()
          .flatMap((r) => [...r.querySelectorAll('button')])
          .some((b) => b.textContent === 'Skip for now'),
      ).toBe(false)
    }
  })

  it('a skipped required secret stays in You (iii cloud still needs a value) with Undo; a skipped optional one is undone from Warnings', async () => {
    const back = await stage(B('setup-fixing-secret-skipped'), { 'iii-cloud::values::skip': {} })
    const row = youItems().find((r) => [...r.querySelectorAll('button')].some((b) => b.textContent === 'Undo'))
    expect(row).toBeTruthy()
    await click([...(row?.querySelectorAll('button') ?? [])].find((b) => b.textContent === 'Undo'))
    expect(back.calls.find((c) => c.id === 'iii-cloud::values::skip')?.payload).toMatchObject({ skip: false })
    expect(row?.textContent).toContain('Undone. The check runs again.')
    const skipped = await stage(B('setup-fixing-skipped'), { 'iii-cloud::values::skip': {} })
    await click(btn('Undo'))
    expect(skipped.calls.find((c) => c.id === 'iii-cloud::values::skip')?.payload).toMatchObject({ skip: false })
    expect(lastCall(skipped.calls)).toMatchObject({ id: 'iii-cloud::check' })
  })

  it('a secret row announces its saved state and its error', async () => {
    await stage(B('deploy-problems'), {
      'iii-cloud::values::save': { environment_id: 'e', results: [{ name: 'STRIPE_SECRET_KEY', ok: true }] },
    })
    const row = youItems()[0]
    await type(row.querySelector('input'), 'v')
    await click(save(row))
    expect(row.querySelector('[role=status]')?.textContent).toContain('Saved')
    await stage(B('deploy-problems'), { 'iii-cloud::values::save': () => Promise.reject(new Error('down')) })
    await type(youItems()[0].querySelector('input'), 'v')
    await click(save(youItems()[0]))
    expect(youItems()[0].querySelector('[role=alert]')).toBeTruthy()
  })
})

describe('the words', () => {
  it('no old word on any state of the check', async () => {
    for (const id of ['deploy-checking', 'deploy-problems', 'deploy-fixing', 'deploy-passed'] as const) {
      await stage(B(id))
      expect(text()).not.toMatch(/app file|Needs you|Worth a look|the run here|Superseded|Cancelled/)
    }
  })
})

describe('the check follows the canvas', () => {
  const noAgent = (s: Status) => ({ ...s, agent: null, handoff_brief: null })
  const primaries = () =>
    [...el.querySelectorAll('button[data-variant=primary]')].filter((b) => !b.hasAttribute('disabled'))

  it('a row names its variable: twenty warnings read as twenty names, not one title', async () => {
    const s = noAgent(rcFixing)
    await stage({ ...s, check: { ...s.check!, problems: rcVariables } })
    const rows = [...(region('The check')?.querySelectorAll('li.iii-cloud-cs-row') ?? [])].map(
      (r) => r.textContent ?? '',
    )
    const named = rows.filter((r) => r.includes('A variable defaults to empty'))
    expect(named.length).toBe(rcVariables.length)
    for (const p of rcVariables) {
      const name = p.detail?.match(/environment\.(\w+)/)?.[1] as string
      expect(named.filter((r) => r.startsWith(`${name}: A variable defaults to empty`))).toHaveLength(1)
    }
    // A warning is not "needs you": it says what it is, with a note mark.
    expect(named.every((r) => r.includes('warning') && !r.includes('needs you'))).toBe(true)
  })

  it("the local run's rows are the canvas's words, with no command", async () => {
    await stage(B('deploy-checking'))
    const t = el.querySelector('[aria-label="Local run"]')?.textContent ?? ''
    expect(t).toContain('Install exactly what the lock file says')
    expect(t).not.toMatch(/Frozen install|Build command|Compose up|--frozen-lockfile/)
  })

  it('"Check again" is said once and no warning panel has a primary action', async () => {
    await stage(B('state-stopped'))
    expect([...el.querySelectorAll('button')].filter((b) => b.textContent === 'Check again')).toHaveLength(1)
    for (const panel of el.querySelectorAll('[role=status]')) {
      expect(panel.querySelector('button[data-variant=primary]')).toBeNull()
    }
    expect(primaries()).toHaveLength(0)
  })

  it('the header says no "Unlocks when" line; the phone bar does, once', async () => {
    await stage(B('deploy-problems'))
    expect(el.querySelector('.iii-cloud-stephead-reason')).toBeNull()
    // Where the canvas says it: the footer.
    expect(text()).toContain('Deploy unlocks when a check passes.')
    await stage(B('deploy-problems'), {}, { narrow: true })
    expect(el.querySelector('.iii-cloud-cs-barnote')?.textContent).toBe('Unlocks when a check passes')
  })

  it('ready: one sub line, "See changes" and "See the file" are links, and the footer says what the canvas says', async () => {
    await stage(B('deploy-review'))
    expect(btn('See changes')).toBeNull()
    expect(el.querySelector('a.iii-cloud-link')?.textContent).toBe('See changes')
    expect(byLink('See the file')?.getAttribute('href')).toContain('/blob/')
    expect(el.querySelectorAll('.iii-cloud-stephead-sub p')).toHaveLength(1)
    expect(text()).toContain('A new commit before you confirm runs the check again.')
    expect(text()).not.toContain('Pinned to the plan')
    expect(el.querySelector('.iii-cloud-stephead-reason')).toBeNull()
    await stage(B('deploy-passed'))
    expect(region('Details')?.textContent).toMatch(/You did \d+ · your agent fixed \d+/)
  })

  it('"Your agent" items are rings, not spinners', async () => {
    await stage(B('deploy-fixing'))
    expect(el.querySelector('li.iii-cloud-cs-item .iii-cloud-ring')).toBeTruthy()
    expect(el.querySelector('li.iii-cloud-cs-item .iii-ui-spin')).toBeNull()
  })
})

const byLink = (label: string) => [...el.querySelectorAll('a')].find((a) => a.textContent === label)
