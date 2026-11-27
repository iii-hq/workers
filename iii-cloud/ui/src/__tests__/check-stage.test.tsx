// The check on the Deploy screen (CheckStage): its header says the next thing to do with the lists' own numbers; You
// has the action in each item (a secret's field, a decision's two ways, the check's words for the rest); Your agent
// starts by itself and keeps Stop on screen; Details is one closed fold; Ready says what the deploy does.
import type { Host } from '@iii-dev/console-ui'
import { act } from 'react'
import { describe, expect, it, type Mock, vi } from 'vitest'
import { LaunchNote } from '../AgentRow'
import { CheckStage, deployLockReason, retryPayload } from '../CheckStage'
import { findings } from '../findings'
import { boards } from '../fixtures'
import { agent, apiPath, base, pr, ready } from '../fixtures/base'
import { found, passed, rcFixing, rcFixingHere, rcVariables } from '../fixtures/check'
import { DEV, live6, serving5 } from '../fixtures/deploy'
import { rcValues, valueItem } from '../fixtures/values'
import { hubOf, useStatus } from '../hub'
import { agentAsks, checklist, handoffMessage, openProblems, runTail, wayOf } from '../model'
import { secretLike, waitingElsewhere } from '../parts'
import type { Route } from '../route'
import type { ChangeEvent, Status } from '../types'
import { copyFlash } from './hooks-mock'
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
      payload: { run: true, from_page: true, push: true, manifest: 'api/worker-compose-prod.yaml' },
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
    const you = Number(/(\d+) things? needs? you/.exec(t)?.[1] ?? (/values? to set/.test(t) ? 1 : 0))
    const agentN = Number(
      /(?:fixing|for your agent to fix)/.test(t)
        ? (/(\d+)(?: for your agent| *$)|fixing (\d+)/.exec(t)?.slice(1).find(Boolean) ?? 0)
        : 0,
    )
    // The values a deploy cannot go without are one more thing, in the values card, never a row of You.
    const values = findings(status).values.length > 0
    expect(you).toBe(youItems().length + (values ? 1 : 0))
    expect(youItems().length ? countOf('You') : undefined).toBe(
      youItems().length ? String(youItems().length) : undefined,
    )
    expect(agentN).toBe(agentLines().length)
    expect(agentN ? countOf('Your agent') : undefined).toBe(agentN ? String(agentN) : undefined)
  })

  it('says only the half that is not zero', async () => {
    await stage(B('deploy-fixing'))
    expect(title()).toBe('Your agent is fixing 1')
    await stage(B('deploy-open-problem'))
    expect(title()).toBe('1 thing needs you · 1 for your agent to fix')
  })

  it('while the check runs, the title is the check', async () => {
    await stage(B('deploy-checking'))
    expect(title()).toBe('Checking billing for dev')
    // A missing secret is never a row of You, nor a name in the check.
    expect(region('You')).toBeNull()
    expect(region('The check')?.textContent).not.toContain('STRIPE_SECRET_KEY')
    const parts = region('The check')
    expect(parts?.querySelector('[aria-label="iii cloud\'s plan"]')).toBeTruthy()
    expect(parts?.querySelector('[aria-label="Local run"]')).toBeTruthy()
    expect(deployButton()?.hasAttribute('disabled')).toBe(true)
  })
})

describe('You', () => {
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
    await stage(B('deploy-open-problem'))
    expect(region('You')?.textContent).not.toContain('never sees')
    expect(region('You')?.textContent).toContain("Your agent can't do these")
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
    // A name is set where it is listed, never reported by the check.
    expect(fold?.querySelector('[aria-label="Warnings"]')?.textContent).not.toContain('SENTRY_DSN')
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

  it('a reused run says when it really ran, and "Run it again" forces a real one', async () => {
    const reused = { ...passed.run, reused: true, finished_at: '2026-11-27T11:30:00.000Z' } as NonNullable<
      typeof passed.run
    >
    const h = await stage({ ...dev, check: { ...passed, run: reused } })
    expect(text()).toContain('the local run passed at')
    expect(text()).toContain('(not run again: nothing changed)')
    await click(btn('Run it again'))
    await flush()
    expect(h.calls.filter((c) => c.id === 'iii-cloud::check').at(-1)?.payload).toMatchObject({
      run: true,
      force: true,
      from_page: true,
    })
    // the Local run part says the same, with the same quiet action
    await stage({ ...{ ...dev, agent: null, handoff_brief: null }, check: { ...passed, ready: false, run: reused } })
    const part = el.querySelector('[aria-label="Local run"]')?.textContent ?? ''
    expect(part).toContain('not run again: nothing changed')
    expect(part).toContain('Run it again')
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
    // The person asked for it here: the worker may wake a waiting agent for it.
    expect(lastCall(h.calls)).toMatchObject({ id: 'iii-cloud::check', payload: { from_page: true } })
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
    expect(lastCall(quiet.calls)).toMatchObject({ id: 'iii-cloud::check', payload: { from_page: true } })
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
      payload: { run: true, from_page: true, manifest: 'api/worker-compose.staging.yaml' },
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

  it('a pick made before any check of the environment does not outlive its first check', async () => {
    const s = B('deploy-passed')
    const h = await stage(s)
    const env = s.check!.environment_id
    // First-time Confirm on a new environment: there was no check to move from, and the pick is its own file.
    await act(async () => hubOf(h.host).setPickedManifest(env, 'api/worker-compose-prod.yaml'))
    expect(hubOf(h.host).movedFrom(env)).toBeNull()
    // The agent took the work to another file since: a later "check again" stays on the work's file.
    expect(retryPayload(h.host, { ...s, work_file: 'api/worker-compose-cloud.yaml' }).manifest).toBe(
      'api/worker-compose-cloud.yaml',
    )
    // While no check of the environment exists yet, the pick is what gets checked.
    expect(retryPayload(h.host, { ...s, check: null, work_file: null }).manifest).toBe('api/worker-compose-prod.yaml')
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

  describe('values have their own card: set, skip or carry on', () => {
    const base = noAgent(rcFixing)
    const answer =
      (items: typeof rcValues.items, over: Partial<typeof rcValues> = {}) =>
      () => ({
        ...rcValues,
        environment_id: base.project?.environment?.id,
        items,
        ...over,
      })
    const card = () => region('Secrets and variables')
    const names = (prefix: string, n: number, over = {}) =>
      Array.from({ length: n }, (_, i) => valueItem(`${prefix}_${i}`, { need: 'optional', suggestions: [], ...over }))
    const quiet = { ...base, check: { ...base.check!, problems: [] } }

    it('twenty optional unset names: counts only, no row anywhere, not "needs you"; Skip for now is one call, then Undo', async () => {
      let skipped = false
      const items = () => names('OPT', 20).map((i) => ({ ...i, skipped }))
      const h = await stage(
        {
          ...base,
          check: {
            ...base.check!,
            problems: rcVariables,
            run: { ...base.check!.run!, status: 'passed' as const, error: null },
          },
        },
        {
          'iii-cloud::values': () => answer(items())(),
          'iii-cloud::values::skip': () => {
            skipped = true
            return { environment_id: 'e', names: [] }
          },
        },
      )
      expect(card()?.textContent).toContain('20 optional, not set')
      expect(card()?.textContent).toContain('Optional ones can wait: you can deploy without them.')
      expect(text()).not.toContain('OPT_0')
      expect(text()).not.toContain('A variable defaults to empty')
      expect(region('You')).toBeNull()
      expect(title() ?? '').not.toMatch(/need|to set/)
      await click(btn('Skip for now'))
      await flush()
      const calls = h.calls.filter((c) => c.id === 'iii-cloud::values::skip')
      expect(calls).toHaveLength(1)
      expect((calls[0].payload as { names: string[]; skip: boolean }).names).toHaveLength(20)
      expect(calls[0].payload).toMatchObject({ skip: true })
      expect(card()?.textContent).toContain('20 skipped for now')
      expect(btn('Undo')).toBeTruthy()
    })

    it('three required names: line 1 with the names, one thing for the person, Set values opens the editor on them', async () => {
      const items = names('REQ', 3, { need: 'required', default: null })
      await stage(quiet, { 'iii-cloud::values': answer(items) })
      expect(card()?.textContent).toContain('3 required by api/worker-compose-prod.yaml and not set')
      for (const i of items) expect(card()?.textContent).toContain(i.name)
      expect(card()?.textContent).toContain(
        'iii cloud does not deploy without them: set them, or leave them empty on purpose.',
      )
      expect(title()).toBe('3 values to set')
      expect(region('You')).toBeNull()
      expect(btn('Set values')?.getAttribute('data-variant')).toBe('secondary')
      await click(btn('Set values'))
      const dialog = el.querySelector('[role=dialog]')?.textContent ?? ''
      for (const i of items) expect(dialog).toContain(i.name)
    })

    it("the agent's secret request is in the card, not in You", async () => {
      const items = [
        valueItem('ASKED_KEY', {
          // The worker marks an ask `required` too.
          need: 'required',
          default: null,
          asked: { request_id: 'r1', description: 'Signs the cookies.' },
          suggestions: [],
        }),
      ]
      await stage(quiet, { 'iii-cloud::values': answer(items) })
      expect(card()?.textContent).toContain('1 your agent asked for')
      expect(card()?.textContent).not.toContain('required by')
      expect(card()?.textContent).toContain('Signs the cookies.')
      expect(region('You')).toBeNull()
    })

    it('a session that cannot write: the buttons are off, with the sentence', async () => {
      await stage(quiet, { 'iii-cloud::values': answer(names('OPT', 2), { save: 'api_key' }) })
      expect((btn('Set values') as HTMLButtonElement).disabled).toBe(true)
      // A skip only writes the worker's own store: it stays on for an API key.
      expect((btn('Skip for now') as HTMLButtonElement).disabled).toBe(false)
      expect(card()?.textContent).toContain("An API key can't write secrets and variables.")
    })

    it('every name set: nothing to do, and the header link goes to the secrets page', async () => {
      const items = names('OK', 2).map((i) => ({ ...i, in_cloud: 'set' as const }))
      const h = await stage(quiet, { 'iii-cloud::values': answer(items) })
      expect(card()?.textContent).toContain('Every name api/worker-compose-prod.yaml reads is set.')
      expect(card()?.textContent).toContain('2 set')
      expect(btn('Set values')).toBeNull()
      await click(btn('All names'))
      expect(h.go).toHaveBeenCalledWith({ at: 'secrets', env: expect.any(String) })
    })

    it('a name set in iii cloud but missing here: its own line, to set or skip, never "every name is set"', async () => {
      const items = [valueItem('STRIPE_KEY', { need: 'required', default: null, in_cloud: 'set', here: 'unset' })]
      const h = await stage(quiet, {
        'iii-cloud::values': answer(items),
        'iii-cloud::values::skip': { environment_id: 'e', names: [] },
      })
      expect(card()?.textContent).toContain('1 set in iii cloud but missing on this machine')
      expect(card()?.textContent).toContain('STRIPE_KEY')
      expect(card()?.textContent).toContain('skip them to run with an empty value')
      expect(card()?.textContent).not.toContain('Every name')
      // "Skip for now" is for the optional names only: here there are none, so it is absent.
      expect(btn('Skip for now')).toBeNull()
      await click(btn('Run without them here'))
      await flush()
      expect(h.calls.find((c) => c.id === 'iii-cloud::values::skip')?.payload).toMatchObject({
        names: ['STRIPE_KEY'],
        skip: true,
      })
      // then a check, which makes the title and the lock true again
      expect(h.calls.find((c) => c.id === 'iii-cloud::check')?.payload).toMatchObject({ from_page: true })
      await click(btn('Set values'))
      expect(el.querySelector('[role=dialog]')?.textContent).toContain('STRIPE_KEY')
    })

    it("the agent's own value problem (the database's) is the agent's, not a value to set, and the hand-off still sees it", async () => {
      const db = {
        ...rcVariables[0],
        id: 'p-db',
        code: 'SECRET_MISSING',
        severity: 'block' as const,
        owner: 'agent' as const,
        title: 'DATABASE_URL has no value',
      }
      const s = {
        ...base,
        check: { ...base.check!, problems: [db], run: { ...base.check!.run!, status: 'passed' as const, error: null } },
      }
      await stage(s)
      expect(title()).toBe('1 for your agent to fix')
      expect(region('Your agent')?.textContent).toContain('DATABASE_URL has no value')
      expect(openProblems(s).map((p) => p.id)).toEqual(['p-db'])
    })

    it('during a hand-off a value row of the brief is neither "fixed" nor shown, and the message counts it', async () => {
      const s = rcFixingHere
      const { rows, fixed } = checklist(s)
      expect(rows.some((r) => /SESSION_SECRET|LOG_LEVEL/.test(r.title))).toBe(false)
      expect(fixed).toBe(checklist(rcFixing).fixed)
      expect(handoffMessage(s)).toContain(`fix the ${openProblems(s, true).length} problems`)
      expect(openProblems(s, true).length).toBeGreaterThan(openProblems(s).length)
    })

    it('a required name can be left empty from the editor, and the dialog speaks for this screen', async () => {
      const items = names('REQ', 1, { need: 'required', default: null })
      await stage(quiet, { 'iii-cloud::values': answer(items) })
      await click(btn('Set values'))
      const dialog = el.querySelector('[role=dialog]')
      expect([...(dialog?.querySelectorAll('option') ?? [])].map((o) => o.textContent)).toContain('Leave empty')
      expect(dialog?.textContent).toContain('iii cloud does not deploy without the required ones')
      expect(dialog?.textContent).not.toContain('The local run needs')
    })

    it('a name set in iii cloud and missing here is never written to iii cloud: no Replace, no typing, no Leave empty, only Skip it here', async () => {
      const items = [
        valueItem('STRIPE_KEY', { need: 'optional', in_cloud: 'set', here: 'unset', updated_at: null }),
        valueItem('OTHER_OPT', { need: 'optional', suggestions: [] }),
      ]
      const h = await stage(quiet, {
        'iii-cloud::values': answer(items),
        'iii-cloud::values::save': { environment_id: 'e', results: [] },
        'iii-cloud::values::skip': { environment_id: 'e', names: [] },
      })
      await click(btn('Set values'))
      const dialog = el.querySelector('[role=dialog]')
      const row = dialog?.querySelector('[data-name="STRIPE_KEY"]')
      expect(row?.textContent).toContain('Set in iii cloud · missing on this machine')
      expect(row?.textContent).toContain('Add it to an env file of your folder (the run reads .env.')
      expect(row?.textContent).not.toMatch(/Replace|Leave empty|Type a value/)
      expect(row?.querySelector('input, textarea')).toBeNull()
      // Even choosing Leave empty for the other name and saving sends nothing about the local one.
      const other = dialog?.querySelector('[data-name="OTHER_OPT"] select') as HTMLSelectElement
      await act(async () => {
        other.value = 'leave_empty'
        other.dispatchEvent(new Event('change', { bubbles: true }))
      })
      await click([...(dialog?.querySelectorAll('button') ?? [])].find((b) => /^Save/.test(b.textContent ?? '')))
      await flush()
      const saves = h.calls.filter((c) => c.id === 'iii-cloud::values::save')
      for (const c of saves) expect(JSON.stringify(c.payload)).not.toContain('STRIPE_KEY')
      await click(btn('Skip it here'))
      expect(h.calls.find((c) => c.id === 'iii-cloud::values::skip')?.payload).toMatchObject({
        names: ['STRIPE_KEY'],
        skip: true,
      })
    })

    it('Skip for now skips only the optional names; once the local ones are skipped the title stops counting them', async () => {
      const local = names('LOC', 3, { need: 'required', default: null, in_cloud: 'set', here: 'unset' })
      const h = await stage(quiet, {
        'iii-cloud::values': answer([...local, ...names('OPT', 12)]),
        'iii-cloud::values::skip': { environment_id: 'e', names: [] },
      })
      await click(btn('Skip for now'))
      const sent = h.calls.find((c) => c.id === 'iii-cloud::values::skip')?.payload as { names: string[] }
      expect(sent.names).toHaveLength(12)
      expect(sent.names.some((n) => n.startsWith('LOC'))).toBe(false)
      const problem = {
        ...rcVariables[0],
        id: 'p-loc',
        code: 'LOCAL_VARIABLE_UNSET',
        source: 'check_here' as const,
        severity: 'block' as const,
        owner: 'person' as const,
        title: 'LOC_0 has no value here',
      }
      const s = { ...base, check: { ...base.check!, problems: [problem] } }
      await stage(s)
      expect(title()).toMatch(/to set/)
      await stage({ ...s, skipped_values: ['LOC_0'] })
      expect(title() ?? '').not.toMatch(/to set/)
    })

    it('when the local run failed the card never says "you can deploy without them", and a missing value explains the stop', async () => {
      // the application starts at `workers`: a missing value can explain a failure there
      const atWorkers = {
        ...quiet,
        check: {
          ...quiet.check!,
          run: { ...quiet.check!.run!, error: { ...quiet.check!.run!.error!, stage: 'workers' as const } },
        },
      }
      await stage(atWorkers, { 'iii-cloud::values': answer(names('OPT', 12)) })
      expect(card()?.textContent).not.toContain('Optional ones can wait')
      expect(card()?.textContent).toContain(
        "The local run failed. If the application needs one of the names above, set it: the run's last lines say which.",
      )
      expect(btn('Set values')).toBeTruthy()
      const problem = {
        ...rcVariables[0],
        id: 'p-loc2',
        code: 'LOCAL_VARIABLE_UNSET',
        source: 'check_here' as const,
        severity: 'block' as const,
        owner: 'person' as const,
        title: 'JWT_SECRET has no value here',
      }
      await stage({ ...base, check: { ...base.check!, problems: [problem] } })
      // The run's own stop would only offer "Run again"; the values card is where it is fixed.
      expect(text()).not.toContain('The local run stopped at')
    })

    it('a skipped value warning, or one on a ready check, is not "fixed" in a hand-off: the brief\'s own code drops it', async () => {
      const b = rcFixing.handoff_brief!
      const warn = (p: (typeof rcVariables)[number]) => ({
        code: p.code,
        owner: p.owner,
        title: p.title,
        file: p.file,
        line: p.line,
        detail: p.detail,
        fix: p.fix,
      })
      const withVars = {
        ...rcFixing,
        handoff_brief: { ...b, person_problems: [...b.person_problems, ...rcVariables.map(warn)] },
      }
      const base2 = checklist(rcFixing)
      // Skipped (info), and then gone from the open list altogether: the rows must not turn into "fixed" ones.
      const skippedAll = {
        ...withVars,
        check: {
          ...withVars.check!,
          problems: withVars.check!.problems.map((p) =>
            p.code === 'EMPTY_DEFAULT_VARIABLE' ? { ...p, skipped: true, severity: 'info' as const } : p,
          ),
        },
      }
      expect(checklist(skippedAll).fixed).toBe(base2.fixed)
      const ready = { ...withVars, check: { ...withVars.check!, ready: true } }
      expect(checklist(ready).rows.some((r) => r.title === rcVariables[0].title)).toBe(false)
    })

    it('the title counts what the card places: an outside-database ask, unreadable values, an ask for a name already set; the badge is the title number', async () => {
      // a. the person chose a database outside iii cloud and the agent asks for its URL
      const db = [
        valueItem('DATABASE_URL', {
          database: true,
          need: 'required',
          default: null,
          asked: { request_id: 'r9', description: 'The database is outside iii cloud.' },
          suggestions: [],
        }),
      ]
      await stage(quiet, { 'iii-cloud::values': answer(db) })
      expect(card()?.textContent).toContain('1 your agent asked for')
      expect(title()).toBe('1 value to set')
      expect(countOf('Secrets and variables')).toBe('1')
      // b. iii cloud's variables could not be read: nothing is known to be missing
      const unknown = names('REQ', 3, { need: 'required', default: null, in_cloud: 'unknown' as const })
      await stage(quiet, { 'iii-cloud::values': answer(unknown, { cloud: 'unreadable' }) })
      expect(card()?.textContent).toContain('could not be read')
      expect(title() ?? '').not.toMatch(/to set/)
      // c. the agent asks for a name that is already set
      const p = B('deploy-problems')
      const asked = findings(p)
        .values.map((i) => i.secret ?? i.name)
        .filter((n): n is string => !!n)
      expect(asked.length).toBeGreaterThan(0)
      const set = asked.map((n) => valueItem(n, { in_cloud: 'set', suggestions: [] }))
      await stage(
        { ...p, check: { ...p.check!, problems: [] } },
        { 'iii-cloud::values': () => ({ ...rcValues, environment_id: p.project?.environment?.id, items: set }) },
      )
      expect(title() ?? '').not.toMatch(/to set/)
      // The badge counts the names that need the person; the optional ones have their own line.
      await stage(quiet, {
        'iii-cloud::values': answer([...names('REQ', 2, { need: 'required', default: null }), ...names('OPT', 5)]),
      })
      expect(countOf('Secrets and variables')).toBe('2')
      expect(card()?.textContent).toContain('5 optional, not set')
    })

    it('"Save N values and check again" asks for the check from the page', async () => {
      const items = [
        valueItem('FROM_FILE', {
          need: 'required',
          default: null,
          suggestions: [{ source: 'env_file', file: '.env', preselect: true, local: false }],
        }),
      ]
      const h = await stage(quiet, {
        'iii-cloud::values': answer(items),
        'iii-cloud::values::save': { environment_id: 'e', results: [{ name: 'FROM_FILE', ok: true }] },
      })
      await click(btn('Set values'))
      await click(
        [...el.querySelectorAll('[role=dialog] button')].find((b) =>
          /^Save \d+ values? and check again/.test(b.textContent ?? ''),
        ),
      )
      await flush()
      expect(h.calls.filter((c) => c.id === 'iii-cloud::check').at(-1)?.payload).toMatchObject({ from_page: true })
    })

    it('the worker\'s own text about the check is a note, not a question: 8 values stay "8 values to set"', async () => {
      const text8 = "Your agent's part is done for now. It waits for 8 values: set them in Secrets and variables."
      const agentNote = {
        ...rcFixing.agent!,
        state: 'waiting_for_you' as const,
        question: text8,
        question_source: 'check' as const,
      }
      const items = names('REQ', 8, { need: 'required', default: null })
      await stage({ ...quiet, agent: agentNote }, { 'iii-cloud::values': answer(items) })
      expect(title()).toBe('8 values to set')
      const t = region('Your agent')?.textContent ?? ''
      expect(t.split(text8).length - 1).toBe(1)
      expect(t).toContain('done for now')
      expect(t).not.toContain('waiting for your answer')
      // a real ask still counts as one more thing; no source: a problem-code prefix is the worker's text
      expect(agentAsks({ ...quiet, agent: { ...agentNote, question_source: 'agent' } })).toBe(true)
      const bare = { ...agentNote, question_source: undefined }
      expect(agentAsks({ ...quiet, agent: { ...bare, question: 'SECRET_MISSING: 8 values' } })).toBe(false)
      expect(agentAsks({ ...quiet, agent: { ...bare, question: 'Keep path://?' } })).toBe(true)
    })

    it('a run that failed at install says values are not the cause, and never "optional ones can wait"', async () => {
      const atRelease = {
        ...quiet,
        check: {
          ...quiet.check!,
          run: {
            ...quiet.check!.run!,
            error: { ...quiet.check!.run!.error!, stage: 'install' as const, code: 'INSTALL_FAILED' },
          },
        },
      }
      await stage(atRelease, { 'iii-cloud::values': answer(names('OPT', 4)) })
      expect(card()?.textContent).toContain('The local run failed at Frozen install; values are not the cause.')
      expect(card()?.textContent).not.toContain('If the application needs')
      expect(card()?.textContent).not.toContain('Optional ones can wait')
    })
    it("a run that failed at the release command may be a value, or the agent's", async () => {
      const atRelease = {
        ...quiet,
        check: {
          ...quiet.check!,
          run: {
            ...quiet.check!.run!,
            error: { ...quiet.check!.run!.error!, stage: 'release' as const, code: 'RELEASE_FAILED' },
          },
        },
      }
      await stage(atRelease, { 'iii-cloud::values': answer(names('OPT', 4)) })
      expect(card()?.textContent).toContain(
        "The local run failed at Release command. If that command reads one of the names above, set it; otherwise it is your agent's.",
      )
      expect(card()?.textContent).not.toContain('values are not the cause')
    })

    it('a mixed check lists the other problems unchanged', async () => {
      const others = base.check!.problems.filter((p) => !rcVariables.some((v) => v.id === p.id))
      await stage({ ...base, check: { ...base.check!, problems: [...others, ...rcVariables] } })
      const before = text()
      await stage({ ...base, check: { ...base.check!, problems: others } })
      expect(text()).toBe(before)
    })
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
    expect(text()).toContain('A new commit before you confirm needs a new check.')
    expect(text()).not.toContain('Pinned to the plan')
    expect(el.querySelector('.iii-cloud-stephead-reason')).toBeNull()
    await stage(B('deploy-passed'))
    expect(region('Details')?.textContent).toMatch(/your agent fixed \d+/)
    // nothing the person did can be counted here: no "You did 0"
    expect(region('Details')?.textContent).not.toContain('You did 0')
  })

  it('"Your agent" items are rings, not spinners', async () => {
    await stage(B('deploy-fixing'))
    expect(el.querySelector('li.iii-cloud-cs-item .iii-cloud-ring')).toBeTruthy()
    expect(el.querySelector('li.iii-cloud-cs-item .iii-ui-spin')).toBeNull()
  })
})

const byLink = (label: string) => [...el.querySelectorAll('a')].find((a) => a.textContent === label)

describe('a branch another environment follows', () => {
  it('Ready says so before Deploy is pressed', async () => {
    const r = B('deploy-review')
    const p0 = r.project as NonNullable<Status['project']>
    // dev follows the default branch, so a deploy moves it to its own branch; another environment already follows that.
    const env = { ...p0.environment!, branch: p0.default_branch! }
    const own = wayOf(r, env)
    const mine = { ...p0, environment: env, environments: p0.environments.map((e) => (e.id === env.id ? env : e)) }
    const target = own.way === 'assisted' && own.own ? own.own : 'x'
    const other = { ...p0.environments[0], id: 'e-other', name: 'staging', branch: target }
    const p = { ...mine, environments: [...mine.environments, other] }
    const s = { ...r, project: p }
    expect(wayOf(s, p.environment!)).toMatchObject({ way: 'assisted', taken: true })
    await stage(s)
    expect(region('What this deploy does')?.textContent).toContain('staging follows')
    expect(region('What this deploy does')?.textContent).toContain('Deploy will refuse')
    // ...and Deploy is off, with the reason beside it, before it is pressed.
    expect((btn('Deploy 9c04e7f to dev') ?? deployButton()) as HTMLButtonElement).toHaveProperty('disabled', true)
    expect(text()).toContain('Deploy will refuse: staging follows')
  })
})

describe('small things the review found', () => {
  it('a held deployment this page started says it waits for the person, not that it is going', () => {
    const r = B('deploy-review')
    const held = {
      ...live6,
      status: 'needs_action' as const,
      outcome: 'running' as const,
      check_id: 'chk-x',
      approved_by_worker: true,
    }
    const s = { ...r, deploys: [held, serving5] }
    expect(deployLockReason(s, DEV.id, { stale: false, sending: false })).toBe(
      'Deployment #6 waits for you: confirm or cancel it first.',
    )
  })

  it('the deploy lock says what a deploy-fix agent does: works, waits, stalls or asks', () => {
    const r = B('deploy-review')
    const lock = (state: 'working' | 'waiting' | 'stalled' | 'waiting_for_you') =>
      deployLockReason(
        {
          ...r,
          agent: { ...agent(state), kind: 'deploy_fix' as const, waiting_for: 'the deploy to end' },
          deploys: [serving5],
        },
        DEV.id,
        { stale: false, sending: false },
      )
    expect(lock('working')).toContain('is fixing a failed deployment')
    expect(lock('waiting')).toContain('waits for the deploy to end')
    expect(lock('stalled')).toContain('stopped making progress')
    expect(lock('waiting_for_you')).toContain('asks about the fix')
    for (const st of ['waiting', 'stalled', 'waiting_for_you'] as const) expect(lock(st)).not.toContain('is fixing')
  })

  it("what the run left or rewrote is a quiet line with the worker's title and its detail, never a warning", async () => {
    const r = B('deploy-passed')
    const note = (code: string, title: string, detail: string) => ({
      ...rcVariables[0],
      id: `p-${code}`,
      code,
      file: null,
      line: null,
      source: 'check_here' as const,
      severity: 'info' as const,
      owner: 'none' as const,
      title,
      detail,
    })
    // The worker's real shapes: the count and the paths are in the detail, the title has no number.
    const left = note(
      'RUN_LEFT_FILES',
      'The local run left files changed in the worktree',
      "The local run left 2 files changed in the agent's copy; they do not count as uncommitted: pnpm-lock.yaml, tsconfig.tsbuildinfo. If one of them is your own edit, commit it. Before a rebase, merge or checkout, restore them; never commit them.",
    )
    const restored = note(
      'RUN_FILES_RESTORED',
      "The local run changed 3 files in the agent's copy; the copy is as it was before the run",
      'Put back: api/a.ts, api/b.ts. Moved: api/c.ts. The rewritten versions are kept in ~/.iii-cloud/run-leftovers.',
    )
    const withNotes = { ...r, check: { ...r.check!, problems: [...r.check!.problems, left, restored] } }
    await stage(withNotes)
    expect(region('Details')?.textContent).toMatch(/Warnings 0/)
    const lines = [...el.querySelectorAll('details.iii-cloud-cs-quiet')]
    expect(lines.map((l) => l.querySelector('summary')?.textContent)).toEqual([left.title, restored.title])
    expect(lines[0].textContent).toContain('pnpm-lock.yaml, tsconfig.tsbuildinfo')
    expect(lines[1].textContent).toContain('Put back: api/a.ts, api/b.ts. Moved: api/c.ts.')
    expect(lines[1].textContent).toContain('run-leftovers')
    await stage(r)
    expect(el.querySelector('details.iii-cloud-cs-quiet')).toBeNull()
  })

  it('a code the page has never heard of (RUN_COMMAND_NOT_IN_IMAGE) is an ordinary agent-owned row, counted', async () => {
    const mk = (id: string, over: object) => ({
      ...rcVariables[0],
      id,
      code: 'RUN_COMMAND_NOT_IN_IMAGE',
      file: null,
      line: null,
      source: 'check_here' as const,
      severity: 'block' as const,
      owner: 'agent' as const,
      title: 'release-control runs with bun, but its runtime is node',
      detail: 'The image has no bun binary.',
      fix: 'Run it with node, or use a runtime image that has bun.',
      ...over,
    })
    const w = { ...rcFixing, agent: null, handoff_brief: null }
    await stage({ ...w, check: { ...w.check!, problems: [mk('p-bun', {})] } })
    expect(title()).toBe('1 for your agent to fix')
    const row = region('Your agent')?.textContent ?? ''
    expect(row).toContain('release-control runs with bun, but its runtime is node')
    expect(row).toContain('The image has no bun binary.')
    await stage({ ...w, check: { ...w.check!, problems: [mk('p-bun2', { detail: null })] } })
    expect(region('Your agent')?.textContent).toContain('Run it with node, or use a runtime image that has bun.')
  })

  it('the title is never empty, and a quiet note without a detail is a plain line', async () => {
    const note = {
      ...rcFixing.agent!,
      state: 'waiting_for_you' as const,
      question: "Your agent's part is done for now.",
      question_source: 'check' as const,
    }
    const w = { ...rcFixing, agent: note }
    await stage({ ...w, check: { ...w.check!, problems: [] } })
    expect((title() ?? '').length).toBeGreaterThan(0)
    const r = B('deploy-passed')
    const bare = {
      ...rcVariables[0],
      id: 'p-bare',
      code: 'RUN_LEFT_FILES',
      file: null,
      line: null,
      source: 'check_here' as const,
      severity: 'info' as const,
      owner: 'none' as const,
      title: 'The local run left files changed in the worktree',
      detail: null,
    }
    await stage({ ...r, check: { ...r.check!, problems: [...r.check!.problems, bare] } })
    expect(el.querySelector('details.iii-cloud-cs-quiet')).toBeNull()
    expect(el.querySelector('p.iii-cloud-cs-quiet')?.textContent).toBe(bare.title)
  })

  it('the worker\'s "Confirm the worktree folder in the chat" text with source "agent" is the person\'s step: counted and shown', async () => {
    const text = 'Confirm the worktree folder in the chat: the agent waits for your answer.'
    const ask = {
      ...rcFixing.agent!,
      state: 'waiting_for_you' as const,
      question: text,
      question_source: 'agent' as const,
    }
    const w = { ...rcFixing, agent: ask }
    const none = {
      ...w,
      check: { ...w.check!, problems: [], run: { ...w.check!.run!, status: 'passed' as const, error: null } },
    }
    await stage(none)
    expect(title()).toBe('1 thing needs you')
    expect(region('Your agent')?.textContent).toContain(text)
    expect(region('Your agent')?.textContent).toContain('waiting for your answer')
  })

  it('COMMAND_PRINTS_ENVIRONMENT is an ordinary agent-owned row', async () => {
    const p = {
      ...rcVariables[0],
      id: 'p-env',
      code: 'COMMAND_PRINTS_ENVIRONMENT',
      file: null,
      line: null,
      source: 'check_here' as const,
      severity: 'block' as const,
      owner: 'agent' as const,
      title: 'The release command prints its whole environment',
      detail: 'It would write every secret into the log.',
      fix: 'Remove the `env` call from the script.',
    }
    const w = { ...rcFixing, agent: null, handoff_brief: null }
    await stage({
      ...w,
      check: { ...w.check!, problems: [p], run: { ...w.check!.run!, status: 'passed' as const, error: null } },
    })
    expect(title()).toBe('1 for your agent to fix')
    expect(region('Your agent')?.textContent).toContain('The release command prints its whole environment')
    expect(region('Your agent')?.textContent).toContain('It would write every secret into the log.')
  })

  it('the values you set since the hand-off began count in "You did N"', async () => {
    const r = B('deploy-passed')
    const names8 = Array.from({ length: 8 }, (_, i) => `SET_${i}_KEY`)
    const brief = {
      ...(rcFixing.handoff_brief ?? ({} as NonNullable<Status['handoff_brief']>)),
      kind: 'ready' as const,
      agent_problems: [],
      person_problems: names8.map((n) => ({
        code: 'SECRET_MISSING',
        owner: 'person' as const,
        title: `${n} has no value`,
        file: null,
        line: null,
        detail: null,
        fix: null,
      })),
    }
    const items = names8.map((n) => valueItem(n, { need: 'required', default: null, in_cloud: 'set', suggestions: [] }))
    const answers = { 'iii-cloud::values': () => ({ ...rcValues, environment_id: r.project?.environment?.id, items }) }
    await stage({ ...r, handoff_brief: brief, agent: { ...rcFixing.agent!, state: 'done' as const } }, answers)
    expect(region('Details')?.textContent).toMatch(/You did 8/)
    // without the read nothing is invented
    await stage({ ...r, handoff_brief: brief, agent: { ...rcFixing.agent!, state: 'done' as const } })
    expect(text()).not.toContain('You did')
  })

  it('a plan that could not be read is said where the plan is, with Check again, and the agent does not start on it', async () => {
    const r = B('deploy-problems')
    const partial = {
      ...r,
      agent: null,
      handoff_brief: null,
      check: {
        ...r.check!,
        plan: { ...r.check!.plan, status: 'unavailable' as const, message: 'The cloud answered 503.' },
      },
    }
    const h = await stage(partial)
    const stopBlock = [...el.querySelectorAll('[role=status]')].find((p) =>
      p.textContent?.includes("iii cloud's plan could not be read"),
    )
    expect(stopBlock?.textContent).toContain('The cloud answered 503.')
    expect([...(stopBlock?.querySelectorAll('button') ?? [])].map((b) => b.textContent)).toEqual(['Check again'])
    expect([...el.querySelectorAll('button')].filter((b) => b.textContent === 'Check again')).toHaveLength(1)
    // the first-time flow: a check that ends with agent problems but no plan does not start the agent
    await act(async () => hubOf(h.host).checking(1))
    await act(async () => hubOf(h.host).checking(-1))
    const ended = { ...partial.check, id: 'chk-partial' }
    await fire(h, { kind: 'check', id: 'chk-partial', terminal: true, summary: '', record: ended } as ChangeEvent)
    await flush()
    expect(handoffs(h)).toHaveLength(0)
  })

  it('the held dot is not shown on the record of that same deployment', () => {
    const r = B('deploy-review')
    const held = { ...live6, status: 'needs_action' as const, outcome: 'running' as const, check_id: 'chk-x' }
    const s = { ...r, deploys: [held, serving5] }
    expect(waitingElsewhere(s, { at: 'env', env: DEV.id })).toBe(false)
    expect(waitingElsewhere(s, { at: 'secrets', env: DEV.id })).toBe(true)
    expect(waitingElsewhere(s, { at: 'env', env: DEV.id, deploy: held.id })).toBe(false)
  })

  it('templates and public keys are not secrets', () => {
    for (const f of ['.env.production.example', 'api/.env.local.sample', 'id_rsa.pub', '.env.example'])
      expect(secretLike(f), f).toBe(false)
    for (const f of ['.env', '.env.production', 'api/.env.local', 'id_rsa', 'deploy.key'])
      expect(secretLike(f), f).toBe(true)
  })
})

describe('an agent that ended its turn waiting on a wake', () => {
  const WAKE = 'test environment values/deploy'
  // A failed run is itself one more thing for the agent: these tests want nothing open for it.
  const passedRun = (st: Status) => ({
    ...st,
    check: { ...st.check!, run: { ...st.check!.run!, status: 'passed' as const, error: null } },
  })
  const waiting = (none = false) => {
    const w = { ...rcFixing, agent: { ...rcFixing.agent!, state: 'waiting' as const, waiting_for: WAKE } }
    return none ? { ...w, check: { ...w.check!, problems: [] } } : w
  }

  it('is not "working": the title, the card and the footer say it waits', async () => {
    await stage(waiting())
    expect(title()).toMatch(new RegExp(`for your agent to fix · it waits: "${WAKE}"$`))
    expect(title()).not.toMatch(/is fixing|checking its fix/)
    const note = region('Your agent')?.textContent ?? ''
    expect(note).toMatch(new RegExp(`waiting \\d+ min: "${WAKE}"`))
    expect(note).not.toMatch(/working \d+ min/)
  })

  it('with nothing for it to fix: "Your agent waits for …"', async () => {
    await stage(passedRun(waiting(true)))
    expect(title()).toBe(`Your agent waits: "${WAKE}"`)
    expect(region('Your agent')?.textContent).toMatch(/waiting \d+ min/)
  })

  it('offers the next step beside the sentence: Open chat as a button, with one line', async () => {
    const openScreen = vi.fn()
    await stage(waiting(), {}, { openScreen })
    const t = region('Your agent')?.textContent ?? ''
    expect(t).toContain('Your agent is not working on this. Tell it in the chat, or check again.')
    const open = [...el.querySelectorAll('button')].filter((b) => b.textContent === 'Open chat')
    expect(open).toHaveLength(1)
    await click(open[0])
    expect(openScreen).toHaveBeenCalled()
    // A working agent has no such line.
    await stage({ ...rcFixing, agent: { ...rcFixing.agent!, state: 'working' as const } }, {}, { openScreen })
    expect(region('Your agent')?.textContent).not.toContain('is not working on this')
  })

  it("the run's stop shows the last meaningful lines, not blanks or separators, six at most", async () => {
    const w = waiting(true)
    const log = [
      '',
      '-----',
      ...Array.from({ length: 8 }, (_, i) => `line ${i}`),
      'JWT_SECRET is empty',
      'REDIS_URL is not set',
      '=====',
      '',
    ]
    await stage({ ...w, check: { ...w.check!, run: { ...w.check!.run!, log_tail: log } } })
    const stop = [...el.querySelectorAll('[role=status]')].find((p) =>
      p.textContent?.includes('The local run stopped at'),
    )
    const t = stop?.textContent ?? ''
    expect(t).toContain('JWT_SECRET is empty')
    expect(t).toContain('REDIS_URL is not set')
    expect(t).not.toContain('-----')
    expect(t).not.toContain('=====')
    expect(t).not.toContain('line 0')
    expect((t.match(/line \d/g) ?? []).length + 2).toBeLessThanOrEqual(6)
  })

  it('a stalled agent is not "fixing", and one with a question is something that needs the person', async () => {
    const st = { ...rcFixing, agent: { ...rcFixing.agent!, state: 'stalled' as const, question: null } }
    await stage(st)
    expect(title()).toMatch(/for your agent to fix · it stopped making progress$/)
    expect(title()).not.toMatch(/is fixing/)
    expect(region('Your agent')?.textContent).not.toMatch(/working \d+ min/)
    expect(region('Your agent')?.textContent).toContain('No progress for')
    const q = {
      ...rcFixing,
      agent: { ...rcFixing.agent!, state: 'waiting_for_you' as const, question: 'Keep path://?' },
    }
    await stage(q)
    expect(title()).toMatch(/^\d+ things? needs? you · \d+ for your agent to fix$/)
    expect(title()).not.toMatch(/is fixing/)
    expect(region('Your agent')?.textContent).toContain('waiting for your answer')
    expect(region('Your agent')?.textContent).not.toMatch(/working \d+ min/)
  })

  it("the line follows what the agent waits for: a running check goes on by itself, a missing value is the values card's, no chat means check again", async () => {
    const openScreen = vi.fn()
    const w = waiting()
    // a. a check is running: the wake fires by itself
    await stage(
      { ...w, check: { ...w.check!, run: { ...w.check!.run!, status: 'running' as const } } },
      {},
      { openScreen },
    )
    expect(region('Your agent')?.textContent).toContain('Your agent goes on when the check ends.')
    expect(region('Your agent')?.textContent).not.toContain('Tell it in the chat')
    // b. nothing open for the agent, the run failed on the person's own value
    const value = {
      ...rcVariables[0],
      id: 'p-own',
      code: 'LOCAL_VARIABLE_UNSET',
      source: 'check_here' as const,
      severity: 'block' as const,
      owner: 'person' as const,
      title: 'JWT_SECRET has no value here',
    }
    const none = waiting(true)
    await stage({ ...none, check: { ...none.check!, problems: [value] } }, {}, { openScreen })
    expect(region('Your agent')?.textContent).toContain(
      'The local run failed, and values are still missing: set them in Secrets and variables above, then check again.',
    )
    expect(el.querySelector('.iii-cloud-cs-nudge button')).toBeNull()
    // c. no chat session: "Check again" alone
    await stage({ ...w, agent: { ...w.agent!, session_id: null } }, {}, { openScreen })
    expect(region('Your agent')?.textContent).toContain('Your agent is not working on this: check again.')
    expect(btn('Open chat')).toBeNull()
    // no activity time: no invented number
    await stage({ ...w, agent: { ...w.agent!, last_activity_at: null } })
    expect(region('Your agent')?.textContent).toMatch(/waiting: "test environment/)
    expect(region('Your agent')?.textContent).not.toMatch(/waiting \d+ min/)
  })

  it('the worker\'s own wake texts read "waits for …", a long label is cut at 80 characters', async () => {
    const w = passedRun(waiting(true))
    await stage({ ...w, agent: { ...w.agent!, waiting_for: 'the check to end' } })
    expect(title()).toBe('Your agent waits for the check to end')
    await stage({ ...w, agent: { ...w.agent!, waiting_for: 'x'.repeat(200) } })
    expect(title()?.length).toBeLessThan(130)
    expect(title()).toContain('…')
  })

  it("the values card names the run's lines only where they are on screen", async () => {
    const base = waiting(true)
    const value = {
      ...rcVariables[0],
      id: 'p-v',
      code: 'LOCAL_VARIABLE_UNSET',
      source: 'check_here' as const,
      severity: 'block' as const,
      owner: 'person' as const,
      title: 'JWT_SECRET has no value here',
    }
    const items = [
      valueItem('JWT_SECRET', { need: 'required', default: null, suggestions: [] }),
      valueItem('OPT_ONE', { suggestions: [] }),
    ]
    const answers = {
      'iii-cloud::values': () => ({ ...rcValues, environment_id: base.project?.environment?.id, items }),
    }
    const err = { ...base.check!.run!.error!, stage: 'workers' as const }
    const s = {
      ...base,
      agent: null,
      check: { ...base.check!, problems: [value], run: { ...base.check!.run!, error: err } },
    }
    // the stop is suppressed while a check_here problem is open: the lines are behind the local run's row
    await stage(s, answers)
    expect(region('Secrets and variables')?.textContent).toContain("open the local run's row below.")
  })

  it('a stalled agent is not told it goes on by itself, says no minutes it cannot know, and names the chat only when one opens', async () => {
    const openScreen = vi.fn()
    const st = { ...rcFixing, agent: { ...rcFixing.agent!, state: 'stalled' as const, question: null } }
    // a check running: a stalled agent has no wake, so no "goes on"
    await stage(
      { ...st, check: { ...st.check!, run: { ...st.check!.run!, status: 'running' as const } } },
      {},
      { openScreen },
    )
    expect(region('Your agent')?.textContent).not.toContain('goes on when the check ends')
    expect(region('Your agent')?.textContent).toContain('open its chat, or stop it')
    // no activity time: no number
    await stage({ ...st, agent: { ...st.agent!, last_activity_at: null } }, {}, { openScreen })
    expect(region('Your agent')?.textContent).toContain('No progress. It may be stuck')
    expect(region('Your agent')?.textContent).not.toMatch(/No progress for/)
    // no chat to open: the card does not send the person there
    await stage({ ...st, agent: { ...st.agent!, session_id: null } })
    expect(region('Your agent')?.textContent).toContain('stop it, or check again')
    expect(region('Your agent')?.textContent).not.toContain('open its chat')
  })

  it('a question stays on screen on a ready check', async () => {
    const r = B('deploy-passed')
    const asking = { ...agent('waiting_for_you'), question: 'Keep path:// for http and storage?' }
    await stage({ ...r, agent: asking })
    expect(title()).toBe('Ready to deploy to dev')
    expect(region('Your agent')?.textContent).toContain('Keep path:// for http and storage?')
  })

  it('an agent that works still says it works', async () => {
    await stage({ ...rcFixing, agent: { ...rcFixing.agent!, state: 'working' as const } })
    expect(title()).toMatch(/your agent is fixing/)
  })
})

describe("the run's last lines", () => {
  it('start at the failure line even when a stack trace comes after, and see through prefixes, colours and box rules', () => {
    const log = [
      '[api] ────────────────',
      '[api] \u001b[31mInvalid environment variables:\u001b[0m',
      '[api]   JWT_SECRET: required',
      '[api]   REDIS_URL: required',
      ...Array.from({ length: 8 }, (_, i) => `    at frame${i} (file.js:${i})`),
      '',
      'Node.js v20.1.0',
      '[31m',
    ]
    const tail = runTail(log)
    expect(tail.length).toBeLessThanOrEqual(6)
    expect(tail.join('\n')).toContain('Invalid environment variables:')
    expect(tail.join('\n')).toContain('JWT_SECRET: required')
    expect(tail).toHaveLength(6)
    // the prefix stays in what is shown; a box rule and a codes-only line are not lines
    expect(tail[0]).toMatch(/^\[api\] /)
    expect(tail.some((l) => l.includes('────') || l.trim() === '[31m')).toBe(false)
  })

  const P = '[worker-release-control] '
  const real = (vars: string[]) => [
    `${P}────────────`,
    `${P}Invalid environment variables:`,
    ...vars.map((v) => `${P}  ${v}: required`),
    `${P}────────────`,
    '',
    `${P}Exiting with error code 1`,
    '!! Workers failed: release-control',
  ]
  const noise = [
    ...Array.from({ length: 10 }, (_, i) => `${P}step ${i}`),
    'WARN missing peer dependency react@18',
    'Registering error handlers',
    'WARN a required option was ignored',
    ...Array.from({ length: 17 }, (_, i) => `${P}more ${i}`),
  ]

  it('the real tail after thirty noisy lines keeps the message and the names', () => {
    const tail = runTail([...noise, ...real(['JWT_SECRET', 'REDIS_URL'])])
    expect(tail.join('\n')).toContain('Invalid environment variables:')
    expect(tail.join('\n')).toContain('JWT_SECRET: required')
    expect(tail.join('\n')).toContain('Exiting with error code 1')
    expect(tail.length).toBeLessThanOrEqual(6)
  })

  it('print, then throw: the line that names the variable survives the stack trace', () => {
    const log = [
      "❌ Invalid environment variables: { JWT_SECRET: [ 'Required' ] }",
      '/app/node_modules/@t3-oss/env-core/dist/index.mjs:30',
      'throw new Error("Invalid environment variables");',
      '^',
      'Error: Invalid environment variables',
      ...Array.from({ length: 6 }, (_, i) => `    at frame${i} (file.js:${i})`),
      'Node.js v20.18.0',
      '!! Workers failed: release-control',
    ]
    const tail = runTail(log)
    expect(tail[0]).toContain('JWT_SECRET')
    expect(tail.length).toBeLessThanOrEqual(6)
    // a name from the values read counts as naming a variable too
    const lower = runTail(
      [
        'Invalid value for jwt_secret',
        'x',
        'throw new Error("boom")',
        ...Array.from({ length: 6 }, (_, i) => `at f${i}`),
        'Node',
        'done',
      ],
      6,
      ['jwt_secret'],
    )
    expect(lower[0]).toContain('jwt_secret')
  })

  it('a later ERROR line or node error code is not a variable: the line naming JWT_SECRET survives the stack', () => {
    const log = [
      "❌ Invalid environment variables: { JWT_SECRET: [ 'Required' ] }",
      '[worker-x] [12:00:01] ERROR (1): Failed to start',
      'Error [ERR_MODULE_NOT_FOUND]: Cannot find module',
      ...Array.from({ length: 6 }, (_, i) => `    at frame${i} (file.js:${i})`),
      'Node.js v20.18.0',
      '!! Workers failed: release-control',
    ]
    expect(runTail(log)[0]).toContain('JWT_SECRET')
    // a name matches as a whole word: `port` is not in "report"
    const t = runTail(
      [
        'Invalid value for port',
        'throw new Error("x")',
        'the report failed: error',
        ...Array.from({ length: 6 }, (_, i) => `at f${i}`),
        'Node',
        'done',
      ],
      6,
      ['port'],
    )
    expect(t[0]).toContain('Invalid value for port')
  })

  it('four missing variables: the message and all four names, six lines, no wasted slot', () => {
    const tail = runTail(real(['A_KEY', 'B_KEY', 'C_KEY', 'D_KEY']))
    expect(tail).toHaveLength(6)
    for (const n of ['A_KEY', 'B_KEY', 'C_KEY', 'D_KEY']) expect(tail.join('\n')).toContain(n)
    expect(tail).not.toContain('…')
  })

  it('no matching line: the last six', () => {
    const tail = runTail(Array.from({ length: 12 }, (_, i) => `plain ${i}`))
    expect(tail).toEqual(Array.from({ length: 6 }, (_, i) => `plain ${i + 6}`))
  })

  it('a colour code without its escape character is not shown', () => {
    const tail = runTail(['[31mInvalid environment variables:[0m', 'X_KEY: required'])
    expect(tail.join('\n')).toBe('Invalid environment variables:\nX_KEY: required')
  })
})

describe("a local run that failed at the release stage is the agent's to fix", () => {
  const note = {
    ...rcFixing.agent!,
    state: 'waiting_for_you' as const,
    question: 'Your agent stopped before running it here like the cloud.',
    question_source: 'check' as const,
  }
  const release = (agent: typeof note | NonNullable<Status['agent']>) => {
    const r = rcFixing.check!
    return {
      ...rcFixing,
      agent,
      check: {
        ...r,
        problems: [],
        run: {
          ...r.run!,
          error: { stage: 'release' as const, code: 'RELEASE_FAILED', message: 'pnpm db:migrate exited 1' },
        },
      },
    } as Status
  }
  const stop = () =>
    [...el.querySelectorAll('[role=status]')].find((p) => p.textContent?.includes('The local run stopped at'))

  it('the title counts it, the stop offers the agent first, and Run again stays second', async () => {
    const openScreen = vi.fn()
    const reused = (sent: boolean) => ({
      'iii-cloud::handoff': { id: 'ho-d', marker: '[m]', reused: true, sent, session_id: 'sess-1' },
    })
    const h = await stage(release(note), reused(true), { openScreen })
    expect(title()).toBe('1 for your agent to fix')
    const buttons = [...(stop()?.querySelectorAll('button') ?? [])].map((b) => b.textContent)
    expect(buttons).toEqual(['Fix with your agent', 'Run again'])
    // The agent waits in its chat: the button calls the worker, which sends what failed into that chat, then the chat opens.
    await click([...(stop()?.querySelectorAll('button') ?? [])][0])
    await flush()
    expect(openScreen).toHaveBeenCalled()
    expect(handoffs(h)).toHaveLength(1)
    expect(handoffs(h)[0]).toMatchObject({ payload: { kind: 'ready' } })
    expect(stop()?.textContent).toContain("Sent to your agent's chat.")
    // The worker could not put it in: the chat opens and the person is told to tell it.
    const openScreen2 = vi.fn()
    await stage(release(note), reused(false), { openScreen: openScreen2 })
    await click([...(stop()?.querySelectorAll('button') ?? [])][0])
    await flush()
    expect(openScreen2).toHaveBeenCalled()
    expect(stop()?.textContent).toContain('Tell it in the chat.')
    expect(stop()?.textContent).not.toContain("Sent to your agent's chat.")
    // No agent at all: the same button sends the hand-off.
    const h2 = await stage({ ...release(note), agent: null, handoff_brief: null }, marker)
    await click([...(stop()?.querySelectorAll('button') ?? [])][0])
    await flush()
    expect(handoffs(h2)).toHaveLength(1)
    // one row for the card, and not a second "Fix with your agent" beside the stop's
    expect(region('Your agent')?.textContent).toContain('The local run stopped at Release')
    expect([...el.querySelectorAll('button')].filter((b) => b.textContent === 'Fix with your agent')).toHaveLength(1)
  })

  it('the title is never missing on the screen of the fifth run: a "done for now" note, no problem rows, a run failed at release', async () => {
    await stage(release(note))
    expect(title()).toBe('1 for your agent to fix')
    expect(region('Your agent')?.textContent).toContain('done for now')
    expect(region('Your agent')?.textContent).toContain('Your agent stopped before running it here like the cloud.')
  })

  const owned = (
    owner: 'agent' | 'person' | 'interrupted',
    over: object = {},
    withheld = false,
    problems: object[] = [],
  ) => {
    const st = release(note)
    return {
      ...st,
      check: {
        ...st.check!,
        problems,
        run: { ...st.check!.run!, error: { ...st.check!.run!.error!, owner, withheld, ...over } },
      },
    } as Status
  }
  const buttonsOf = () => [...(stop()?.querySelectorAll('button') ?? [])].map((b) => b.textContent)

  it("the worker's owner decides: agent counts and offers the agent", async () => {
    await stage(owned('agent'))
    expect(title()).toBe('1 for your agent to fix')
    expect(buttonsOf()).toEqual(['Fix with your agent', 'Run again'])
  })

  it("person: a tool missing on this machine is the person's, with no offer and no count", async () => {
    await stage(
      owned('person', { code: 'TOOL_MISSING', stage: 'install', message: '`pnpm` is not installed or not on PATH' }),
    )
    expect(title() ?? '').not.toContain('for your agent to fix')
    expect(stop()?.textContent).toContain("This one is yours to fix here, not your agent's.")
    expect(buttonsOf()).toEqual(['Run again'])
  })

  it('interrupted: a crash of the worker only offers Run again', async () => {
    await stage(owned('interrupted', { code: 'RUN_PANIC', message: 'The run stopped unexpectedly.' }))
    expect(title() ?? '').not.toContain('for your agent to fix')
    expect(stop()?.textContent).toContain('interrupted')
    expect(buttonsOf()).toEqual(['Run again'])
  })

  it("withheld: the person's open thing comes first, no offer and no count", async () => {
    const need = {
      ...rcVariables[0],
      id: 'p-cred',
      code: 'LITERAL_CREDENTIAL',
      file: null,
      line: null,
      source: 'cloud_plan' as const,
      severity: 'block' as const,
      owner: 'person' as const,
      title: 'A literal credential is in the compose file',
      detail: null,
    }
    await stage(owned('agent', {}, true, [need]))
    expect(stop()?.textContent).toContain('Your part comes first: A literal credential is in the compose file.')
    expect(buttonsOf()).toEqual(['Run again'])
    expect(region('Your agent')?.textContent ?? '').not.toContain('The local run stopped at Release')
  })

  it('a real question open: no "Fix with your agent", answer it first; a worker "asks" answer says the same', async () => {
    const asking = { ...note, question: 'Which database should the migration use?', question_source: 'agent' as const }
    const openScreen = vi.fn()
    await stage(release(asking), {}, { openScreen })
    expect(buttonsOf()).toEqual(['Open chat', 'Run again'])
    expect(stop()?.textContent).toContain("Answer your agent's question in the chat first.")
    await click([...(stop()?.querySelectorAll('button') ?? [])][0])
    expect(openScreen).toHaveBeenCalled()
    // the status shows no question, but the worker says one is open
    const reply = {
      'iii-cloud::handoff': { id: 'ho-d', marker: '[m]', reused: true, sent: false, asks: true, session_id: 'sess-1' },
    }
    const quietNote = { ...note, check: undefined }
    await stage({ ...release(quietNote), check: { ...release(note).check!, id: 'chk-asks' } }, reply, {
      openScreen: vi.fn(),
    })
    await click([...(stop()?.querySelectorAll('button') ?? [])][0])
    await flush()
    expect(stop()?.textContent).toContain("Answer your agent's question in the chat first.")
    expect(stop()?.textContent).not.toContain("Sent to your agent's chat.")
  })

  it("after a page check ends the button waits six seconds for the worker's own continue", async () => {
    const h = await stage(release(note), {}, { openScreen: vi.fn() })
    await act(async () => {
      hubOf(h.host).checking(1)
      hubOf(h.host).checking(-1)
    })
    const first = () => [...(stop()?.querySelectorAll('button') ?? [])][0] as HTMLButtonElement
    expect(first().disabled).toBe(true)
    expect(stop()?.textContent).toContain('Checking whether your agent took it…')
  })

  it("with a hand-off open the call carries no text (the worker's continue is the one message); without one it does", async () => {
    const reused = {
      'iii-cloud::handoff': { id: 'ho-d', marker: '[m]', reused: true, sent: true, session_id: 'sess-1' },
    }
    const h = await stage(release(note), reused, { openScreen: vi.fn() })
    await click([...(stop()?.querySelectorAll('button') ?? [])][0])
    await flush()
    expect(handoffs(h)).toHaveLength(1)
    expect(h.calls.find((c) => c.id === 'iii-cloud::handoff')?.payload).not.toHaveProperty('text')
    const h2 = await stage({ ...release(note), agent: null, handoff_brief: null }, marker)
    await click([...(stop()?.querySelectorAll('button') ?? [])][0])
    await flush()
    expect(h2.calls.find((c) => c.id === 'iii-cloud::handoff')?.payload).toHaveProperty('text')
  })

  it('one voice: no "Tell it in the chat, or check again" beside the stop\'s Fix button, and a failed copy says so', async () => {
    const w = release({ ...note, state: 'waiting' as const, waiting_for: 'the check to end' } as never)
    const r = w.check!
    await stage({ ...w, check: { ...r, run: { ...r.run!, log_path: '/home/you/.iii-cloud/runs/x.log' } } })
    expect(region('Your agent')?.textContent).not.toContain('Tell it in the chat, or check again')
    expect(buttonsOf()).toContain('Fix with your agent')
    copyFlash.state = 'failed'
    await stage({ ...w, check: { ...r, run: { ...r.run!, log_path: '/home/you/.iii-cloud/runs/x.log' } } })
    expect(stop()?.textContent).toContain('Could not copy: select the text.')
    copyFlash.state = 'idle'
  })

  it('the "sent" sentence belongs to the check it was pressed for: a newer failure shows nothing stale', async () => {
    const reused = {
      'iii-cloud::handoff': { id: 'ho-d', marker: '[m]', reused: true, sent: true, session_id: 'sess-1' },
    }
    await stage(release(note), reused, { openScreen: vi.fn() })
    await click([...(stop()?.querySelectorAll('button') ?? [])][0])
    await flush()
    expect(stop()?.textContent).toContain("Sent to your agent's chat.")
    // an hour later another check fails at Release: nothing was sent for it
    const next = release(note)
    await stage({ ...next, check: { ...next.check!, id: 'chk-newer' } })
    expect(stop()?.textContent).toContain('stopped at Release')
    expect(stop()?.textContent).not.toContain("Sent to your agent's chat.")
  })

  it('an agent already on it: "Your agent is on it." and no button for it', async () => {
    await stage(release({ ...rcFixing.agent!, state: 'working' as const }))
    expect(stop()?.textContent).toContain('Your agent is on it.')
    expect([...(stop()?.querySelectorAll('button') ?? [])].map((b) => b.textContent)).toEqual(['Run again'])
  })

  it("the run's whole log is one quiet line with a copy button under its last lines", async () => {
    const LOG = '/home/you/.iii-cloud/runs/run-1.log'
    const r = rcFixing.check!
    const w = {
      ...rcFixing,
      agent: null,
      handoff_brief: null,
      check: { ...r, problems: [], run: { ...r.run!, log_path: LOG } },
    } as Status
    await stage(w)
    const stopBlock = [...el.querySelectorAll('[role=status]')].find((p) =>
      p.textContent?.includes('The local run stopped at'),
    )
    expect(stopBlock?.textContent).toContain(`The whole log: ${LOG}`)
    expect([...(stopBlock?.querySelectorAll('button') ?? [])].map((b) => b.textContent)).toContain('Copy')
    await stage({ ...w, check: { ...w.check!, run: { ...w.check!.run!, log_path: null } } })
    expect(text()).not.toContain('The whole log')
  })
})
