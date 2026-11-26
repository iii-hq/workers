// The check before deploy (U2, CheckStage): the four states, both parts, "Only you can do this", what the check
// found, the checks so far, the actions, the Dep7 panels, the auto-agent checkbox, and the agent hand-off; then the
// 26/11 redesign (CHECKSTAGE-PLAN D1–D9): collapsible sections, grouped problems, one summary that adds up.
import type { Host } from '@iii-dev/console-ui'
import { act } from 'react'
import { describe, expect, it, type Mock, vi } from 'vitest'
import { LaunchNote } from '../AgentRow'
import { clearAsked, clock } from '../asked'
import { CheckStage } from '../CheckStage'
import { boards } from '../fixtures'
import { agent, agentStatus, base, run as baseRun, failedDeploy, ready } from '../fixtures/base'
import {
  configPreview,
  DEV,
  found,
  passed,
  RC_HERE_UNSET,
  RC_SKIPPED,
  RC_VARIABLES,
  rcAgent,
  rcFixing,
  rcSdk,
  rights,
  stripe,
  stripeAsk,
  TEST_ENV,
  webOverride,
} from '../fixtures/check'
import { rcHereValues, rcValues, valueItem } from '../fixtures/values'
import { handOff } from '../handoff'
import { useStatus } from '../hub'
import { fixMessage, handoffMessage } from '../model'
import type { Route } from '../route'
import type { BranchRef, ChangeEvent, CommitLine, Status } from '../types'
import { fakeHost } from './host'
import { btn, byText, click, el, flush, lastCall, root, text, type, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

/** The stage as an area mounts it, on the hub's live status, with the launch note CloudPage shows above it. */
function Live({
  narrow = false,
  ...props
}: {
  host: Host
  go: (r: Route) => void
  scenario: 'setup' | 'change'
  checkId?: string
  narrow?: boolean
}) {
  const { status } = useStatus(props.host, { poll: false })
  return status ? (
    <>
      <LaunchNote />
      <CheckStage {...props} status={status} narrow={narrow} />
    </>
  ) : null
}

type Opts = {
  scenario?: 'setup' | 'change'
  checkId?: string
  chat?: Record<string, unknown>
  openScreen?: Mock
  narrow?: boolean
}
let mounts = 0
async function stage(
  status: Parameters<typeof fakeHost>[0],
  answers: Parameters<typeof fakeHost>[1] = {},
  opts: Opts = {},
) {
  const h = fakeHost(status, answers)
  const host = h.host as unknown as { chat: unknown; panels: Record<string, unknown> }
  if (opts.chat) host.chat = opts.chat
  if (opts.openScreen) host.panels.openScreen = opts.openScreen
  const go = vi.fn()
  mounts += 1
  await act(async () =>
    root.render(
      <Live
        key={mounts}
        host={h.host}
        go={go}
        scenario={opts.scenario ?? 'change'}
        checkId={opts.checkId}
        narrow={opts.narrow}
      />,
    ),
  )
  await flush()
  return { ...h, go }
}

const B = (id: keyof typeof boards) => boards[id].status
/** production's check passed, with the run here. */
const ran: Status = { ...base, check: ready }
/** production's hand-off to the agent, in `state`. */
const agentAt = (state: Parameters<typeof agent>[0]): Status => ({ ...agentStatus, agent: agent(state) })
const fire = (h: ReturnType<typeof fakeHost>, ev: ChangeEvent) =>
  act(async () => h.handlers.get('iii::iii-cloud-ui::events::console-test')?.(ev))
const title = () => el.querySelector('h2')?.textContent
const chip = () => el.querySelector('.iii-cloud-stephead [aria-live=polite]')?.textContent
const region = (name: string) => el.querySelector(`[role=region][aria-label="${name}"]`)
/** A section's rows (a group's own lines left out: they are inside its row). */
const rowsOf = (name: string) =>
  [...(region(name)?.querySelectorAll('li') ?? [])]
    .filter((r) => !r.parentElement?.classList.contains('iii-cloud-cs-each'))
    .map((r) => r.textContent ?? '')
/** The section's header (its trigger): aria-expanded, count chip and note. */
const head = (name: string) => region(name)?.querySelector('button.iii-cloud-cs-head')
const noteOf = (name: string) => head(name)?.querySelector('.iii-cloud-cs-note')?.textContent
const countOf = (name: string) => head(name)?.querySelector('[data-chip]')?.textContent
const marksOf = (name: string) =>
  [...(region(name)?.querySelectorAll('li.iii-cloud-cs-item > [role=img]') ?? [])].map((m) =>
    m.getAttribute('aria-label'),
  )
const panel = (headline: string) =>
  [...el.querySelectorAll('[role=status]')].find((p) => p.querySelector('b')?.textContent === headline)
const handoffs = (h: { calls: { id: string }[] }) => h.calls.filter((c) => c.id === 'iii-cloud::handoff')
const locked = () => {
  const b = btn('Review and deploy')
  return !b || b.hasAttribute('disabled')
}
const marker = { 'iii-cloud::handoff': { id: 'ho-9', marker: '[iii-cloud handoff ho-9]' } }

describe('checking', () => {
  it('both parts, live; "You can leave this page"; Review locked with its note', async () => {
    await stage(B('deploy-checking'))
    expect(title()).toBe('Checking before deploy')
    expect(chip()).toBe('Checking')
    expect(text()).toMatch(/Commit e41a7c3 · in a separate copy of your code · \d+ s so far/)
    expect(text()).toContain('App file api/worker-compose-prod.yaml · the file dev runs')
    const cloud = rowsOf('iii cloud accepts it')
    expect(region('iii cloud accepts it')?.textContent).toContain('2 to fix')
    expect(cloud[0]).toContain('The new search worker has no pinned version')
    expect(cloud[0]).toContain('needs your agent')
    expect(cloud[1]).toContain('needs you')
    const here = rowsOf('It runs here like the cloud')
    expect(here).toHaveLength(7)
    expect(here[0]).toContain('Frozen install')
    expect(here[0]).toContain('6.1s')
    expect(here[2]).toContain('running')
    expect(here[3]).toContain('waiting')
    expect(region('It runs here like the cloud')?.textContent).toMatch(/running · \d+ s/)
    expect(region('It runs here like the cloud')?.querySelectorAll('[role=img][data-mark=work]')).toHaveLength(1)
    expect(text()).toContain('You can leave this page. The console tells you when the check is done.')
    expect(locked()).toBe(true)
    expect(text()).toContain('Unlocks when a check passes on the commit you deploy.')
    expect(region('What the check found')).toBeNull()
  })

  it('no check yet: Check now runs it with the run; skeleton parts until it reports', async () => {
    let release: (v: unknown) => void = () => undefined
    const gate = new Promise((r) => {
      release = r
    })
    const h = await stage({ ...B('deploy-checking'), check: null }, { 'iii-cloud::check': () => gate })
    expect(text()).toContain('Not checked yet')
    await click(btn('Check now'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
    expect(title()).toBe('Checking before deploy')
    expect(region('iii cloud accepts it')?.querySelectorAll('.iii-cloud-cs-skel').length).toBeGreaterThan(0)
    expect(region('It runs here like the cloud')?.querySelectorAll('.iii-cloud-cs-skel').length).toBeGreaterThan(0)
    await act(async () => release(found))
    await flush()
  })
})

describe('problems', () => {
  it('the title counts everything to fix; each row says where and why, with the log of the run here', async () => {
    await stage(B('deploy-problems'))
    expect(title()).toBe('3 things to fix before dev can get this')
    expect(chip()).toBe('Not ready')
    expect(text()).toContain('Commit e41a7c3 · checked on iii cloud and here')
    const rows = rowsOf('What the check found')
    expect(rows).toHaveLength(2)
    expect(rows[0]).toContain('The new search worker has no pinned version')
    expect(rows[0]).toContain('iii cloud')
    // D7: who handles it is said once, in the section's header, not on every row.
    expect(rows[0]).not.toContain('needs your agent')
    expect(noteOf('What the check found')).toBe('your agent fixes these in a separate copy')
    expect(rows[0]).toContain('The cloud builds only pinned versions')
    expect(rows[1]).toContain('here')
    expect(rows[1]).toContain('0008_trgm.sql installs pg_stat_statements')
    const log = region('What the check found')?.querySelector('details')
    expect(log?.querySelector('summary')?.textContent).toBe('Last lines of the run here')
    expect(log?.textContent).toContain('permission denied to create extension')
    expect(countOf('Needs you')).toBe('1')
    expect(region('Needs you')?.textContent).toContain('the agent never sees secrets')
    expect(region('Needs you')?.textContent).toContain('STRIPE_SECRET_KEY')
    expect(region('It runs here like the cloud')).toBeNull()
    expect(btn('Review and deploy')).toBeNull()
  })

  it('a secret the person gives: Set values opens the values; the value goes only to values::save, then it checks again', async () => {
    const spies = (['log', 'info', 'warn', 'error', 'debug'] as const).map((k) =>
      vi.spyOn(console, k).mockImplementation(() => undefined),
    )
    const stripeValue = valueItem('STRIPE_SECRET_KEY', {
      need: 'required',
      default: null,
      containers: ['billing'],
      line: 31,
      asked: { request_id: stripeAsk.id, description: stripeAsk.description },
    })
    const values = { ...rcValues, environment_id: DEV.id, environment: 'dev', items: [stripeValue] }
    const saved = { environment_id: DEV.id, results: [{ name: 'STRIPE_SECRET_KEY', ok: true }] }
    const h = await stage(B('deploy-problems'), { 'iii-cloud::values': values, 'iii-cloud::values::save': saved })
    expect(region('Needs you')?.querySelector('input')).toBeNull()
    await click(byText('[role=region][aria-label="Needs you"] button', 'Set values'))
    const dialog = el.querySelector('[role=dialog]')
    expect(dialog?.querySelector('h2')?.textContent).toBe('Values for dev')
    const input = dialog?.querySelector('input[aria-label="Value of STRIPE_SECRET_KEY"]') as HTMLInputElement
    expect(input.type).toBe('password')
    await type(input, 'sk_live_TOPSECRET')
    await click(btn('Save 1 value and check again'))
    await flush()
    expect(h.calls.filter((c) => c.id === 'iii-cloud::values::save').map((c) => c.payload)).toEqual([
      {
        environment_id: DEV.id,
        manifest: rcValues.manifest,
        items: [{ name: 'STRIPE_SECRET_KEY', from: 'typed', type: 'secret', value: 'sk_live_TOPSECRET' }],
      },
    ])
    expect(JSON.stringify(h.calls.filter((c) => c.id !== 'iii-cloud::values::save'))).not.toContain('TOPSECRET')
    for (const s of spies) expect(JSON.stringify(s.mock.calls)).not.toContain('TOPSECRET')
    expect(el.querySelector('[role=dialog]')).toBeNull()
    expect(el.innerHTML).not.toContain('TOPSECRET')
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
    for (const s of spies) s.mockRestore()
  })

  it('Set values with nothing chosen: Save is off, and Cancel closes it with no write and no check', async () => {
    const h = await stage(B('deploy-problems'), { 'iii-cloud::values': { ...rcValues, items: [] } })
    await click(byText('[role=region][aria-label="Needs you"] button', 'Set values'))
    expect(btn('Save and check again')?.hasAttribute('disabled')).toBe(true)
    await click(btn('Cancel'))
    expect(el.querySelector('[role=dialog]')).toBeNull()
    expect(
      h.calls.map((c) => c.id).filter((id) => id === 'iii-cloud::values::save' || id === 'iii-cloud::check'),
    ).toEqual([])
  })

  it('DATABASE_URL is never asked for, a not-needed one stays hidden, an external database is asked for', async () => {
    const unset = (id: string, name: string) => ({
      ...stripe,
      id,
      title: `${name} is not set`,
      detail: `${name} has no value.`,
    })
    const s = B('deploy-problems')
    const problems = [
      ...s.check!.problems.filter((p) => p !== stripe),
      unset('db', 'DATABASE_URL'),
      unset('ai', 'OPENAI_API_KEY'),
    ]
    const check = { ...s.check!, problems }
    const notNeeded = {
      ...stripeAsk,
      id: 'sr-ai',
      name: 'OPENAI_API_KEY',
      mode: 'not_needed' as const,
      state: 'noted' as const,
    }
    const dbAsk = { ...stripeAsk, id: 'sr-db', name: 'DATABASE_URL' }
    await stage({ ...s, check, secrets: [notNeeded, dbAsk] })
    expect(region('Needs you')).toBeNull()
    expect(title()).toBe('2 things to fix before dev can get this')
    await stage({ ...s, check, secrets: [notNeeded, { ...dbAsk, external: true }] })
    const you = region('Needs you')
    expect(you?.textContent).toContain('DATABASE_URL')
    expect(you?.textContent).not.toContain('OPENAI_API_KEY')
    expect(byText('[role=region][aria-label="Needs you"] button', 'Set values')).toBeTruthy()
    expect(title()).toBe('3 things to fix before dev can get this')
  })

  it('Fix with your agent hands the check off (iii-cloud::handoff, kind ready), then says when it asked', async () => {
    const h = await stage(B('deploy-problems'), marker)
    expect(text()).toContain('Opens its chat beside this page. The check runs again after each fix.')
    await click(btn('Fix with your agent'))
    expect(handoffs(h)).toHaveLength(1)
    expect(handoffs(h)[0]).toMatchObject({ payload: { kind: 'ready', check_id: 'chk-d1' } })
    expect(h.compose.mock.calls[0][0].text.split('\n')[0]).toBe('[iii-cloud handoff ho-9]')
    expect(text()).toMatch(/Asked your agent at .+\. This page updates when it checks again\./)
    expect(btn('Start over with your agent')?.getAttribute('data-variant')).toBe('secondary')
  })

  it('an info problem is listed without an owner and is not counted', async () => {
    const s = base
    const check = { ...s.check!, problems: [{ ...s.check!.problems[0], owner: 'none' as const }] }
    await stage({ ...s, check })
    expect(title()).toBe('Not ready for production yet')
    // A note is only worth a look: its own section, folded.
    expect(region('What the check found')).toBeNull()
    expect(head('Worth a look')?.getAttribute('aria-expanded')).toBe('false')
    const rows = rowsOf('Worth a look')
    expect(rows).toHaveLength(1)
    expect(rows[0]).toContain('Lock missing')
    expect(rows[0]).not.toContain('needs')
  })
})

describe('cloud check pending', () => {
  const pend = (status: 'not_pushed' | 'commit_not_planned') => {
    const s = base
    return { ...s, check: { ...s.check!, plan: { ...s.check!.plan, status } } }
  }
  const handedOff = (status: 'not_pushed' | 'commit_not_planned') => ({ ...pend(status), agent: agent('working') })
  it('not_pushed: the count is never presented as complete; with a hand-off, the agent pushes', async () => {
    await stage(pend('not_pushed'))
    expect(title()).toBe('At least 4 things to fix before production can get this')
    expect(text()).toContain('Commit 3c4d2e0 · iii cloud check pending')
    await stage(handedOff('not_pushed'))
    expect(text()).toContain(
      "iii cloud hasn't checked this commit yet: your agent pushes the branch, then checks again.",
    )
  })

  it('no hand-off: Check again pushes the fix branch first, and says so', async () => {
    const s = pend('not_pushed')
    const h = await stage({ ...s, check: { ...s.check!, problems: [] } }, { 'iii-cloud::check': {} })
    expect(panel('iii cloud check pending')?.textContent).toContain(
      'iii cloud checks a commit once it is on GitHub: Check again pushes iii-cloud/ready-production first.',
    )
    expect(text()).not.toContain('your agent pushes')
    await click(btn('Check again'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
  })

  it('no hand-off, no fix branch (the checkout): no promise to push, and Check again pushes nothing', async () => {
    const s = pend('commit_not_planned')
    const p = s.project!
    const co = {
      ...s,
      check: { ...s.check!, problems: [] },
      project: { ...p, workspace: { ...p.workspace!, mode: 'checkout' as const, branch: 'main' } },
    }
    const h = await stage(co, { 'iii-cloud::check': {} })
    expect(panel('iii cloud check pending')?.textContent).toContain('iii cloud checks a commit once it is on GitHub.')
    expect(text()).not.toContain('pushes')
    await click(btn('Check again'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true } })
  })

  it('commit_not_planned says the cloud cannot see the commit yet, not that only branch commits are planned', async () => {
    await stage(handedOff('commit_not_planned'))
    expect(text()).toContain(
      "iii cloud can't see this commit in the repository yet. Your agent pushes the branch, then checks again.",
    )
    expect(text()).not.toContain('only checks commits')
    expect(panel('iii cloud check pending')).toBeTruthy()
    await stage(pend('commit_not_planned'))
    expect(title()).toMatch(/^At least /)
  })
})

describe('fixing', () => {
  it('N fixed, the checks so far from status.checks, Review waiting for the running check', async () => {
    await stage(B('deploy-fixing'))
    expect(title()).toBe('Your agent is fixing what the check found')
    expect(chip()).toBe('Checking again')
    expect(text()).toContain('3 problems: 0 need you · your agent is on 1 · 2 fixed')
    expect(text()).not.toMatch(/nothing is pushed/i)
    // P4: what the agent is on, open and spinning; everything fixed, both lists, folded in its own section.
    expect(marksOf('Your agent is fixing')).toEqual(['Your agent is working on this'])
    expect(head('Your agent is fixing')?.getAttribute('aria-expanded')).toBe('true')
    expect(region('Needs you')).toBeNull()
    expect(countOf('Fixed')).toBe('2')
    expect(head('Fixed')?.getAttribute('aria-expanded')).toBe('false')
    expect(rowsOf('Fixed').some((r) => r.includes('Stripe secret key for dev'))).toBe(true)
    expect(marksOf('Fixed')).toEqual(['fixed', 'fixed'])
    const past = rowsOf('Checks so far')
    expect(past).toHaveLength(3)
    expect(past[0]).toContain('e41a7c3')
    expect(past[0]).toContain('Your commits · 3 problems')
    expect(past[1]).toContain('3f1b2aa')
    expect(past[1]).toContain('After a fix · 1 problem')
    expect(past[2]).toContain('9c04e7f')
    expect(past[2]).toMatch(/After a fix · running\d+ s so far/)
    expect(region('Checks so far')?.textContent).toContain('Nothing is deployed while it works.')
    expect(locked()).toBe(true)
    expect(text()).toContain('Waiting for check 3.')
    expect(el.querySelector('.iii-cloud-agent-head')?.textContent).toContain('Working')
  })

  it('setup: the first check is "Your code"; a secret still open stays under Needs you', async () => {
    await stage(B('setup-fixing'), {}, { scenario: 'setup' })
    expect(rowsOf('Checks so far')[0]).toContain('Your code · ')
    expect(byText('[role=region][aria-label="Needs you"] button', 'Set values')).toBeTruthy()
    expect(rowsOf('Your agent is fixing').some((r) => r.includes('Stripe secret key'))).toBe(false)
    expect(text()).toContain('3 problems: 1 needs you · your agent is on 1 · 1 fixed')
  })
})

describe('passed', () => {
  it('Review and deploy goes to the review step (setup: to ship); only the commit that passed', async () => {
    const h = await stage(B('deploy-passed'))
    expect(title()).toBe('Ready to deploy to dev')
    expect(chip()).toBe('Passed')
    expect(text()).toMatch(/9c04e7f passed on iii cloud and here at /)
    expect(text()).toContain('App file api/worker-compose-prod.yaml · the file dev runs · 9 workers in 3 groups')
    // Nothing left open: only what was fixed, folded.
    expect(region('What the check found')).toBeNull()
    expect(head('Fixed')?.getAttribute('aria-expanded')).toBe('false')
    expect(rowsOf('Checks so far').at(-1)).toContain('After a fix · passed')
    expect(text()).toContain('Only the commit that passed can be deployed.')
    expect(btn('Review and deploy')?.getAttribute('data-variant')).toBe('primary')
    await click(btn('Review and deploy'))
    // The screen picks its step from the state: a passed check is the review.
    expect(h.go).toHaveBeenCalledWith({ at: 'deploy', env: DEV.id })
  })

  it('Review is never enabled while the check is not ready', async () => {
    for (const id of [
      'deploy-checking',
      'deploy-problems',
      'deploy-fixing',
      'state-stopped',
      'state-docker',
    ] as const) {
      await stage(B(id))
      expect(locked()).toBe(true)
    }
  })

  it('with nothing found, both parts show what passed: the stages, what the run does not cover', async () => {
    await stage(ran)
    for (const t of [
      'Frozen install',
      'Build command',
      'Release command',
      'Local Postgres with pgvector, for this run only',
      'Compose up: registry, http, storage',
      '3 of 3',
      '/api',
      '200',
      "Not covered here: The cloud's build container",
      'Engine 0.27.0.',
    ])
      expect(text()).toContain(t)
  })

  it('a dirty run says uncommitted changes, not another commit', async () => {
    const s = ran
    await stage({ ...s, check: { ...s.check!, run: { ...s.check!.run!, commit: `${s.check!.commit}+dirty` } } })
    expect(text()).toContain('3c4d2e0 passed')
    const f = base
    await stage({ ...f, check: { ...f.check!, run: { ...ran.check!.run!, commit: `${f.check!.commit}+dirty` } } })
    expect(text()).toContain('Commit 3c4d2e0 with uncommitted changes')
    expect(text()).not.toContain('+dirty')
  })
})

describe('when the check cannot finish (Dep7)', () => {
  it("Docker isn't running: Run again checks with the run; the Docker docs", async () => {
    const h = await stage(B('state-docker'))
    const p = panel("Docker isn't running on this machine")
    expect(p?.getAttribute('data-variant')).toBe('alert')
    expect(p?.textContent).toContain('The cloud part of the check still ran: 2 problems.')
    expect(p?.querySelector('a')?.getAttribute('href')).toBe('https://docs.docker.com/get-started/get-docker/')
    await click(btn('Run again'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
  })

  it('RUN_BUSY says since when the run is going', async () => {
    const at = '2026-11-25T14:03:00.000Z'
    const busy = Object.assign(new Error('busy'), { code: 'RUN_BUSY', started_at: at })
    await stage(B('state-docker'), { 'iii-cloud::check': busy })
    await click(btn('Run again'))
    await flush()
    expect(panel(`Already running since ${clock(at)}`)?.textContent).toContain('This page updates when the run ends.')
  })

  it('another run failure with no problem here is worded plainly, with Run again', async () => {
    const s = ran
    const run = {
      ...s.check!.run!,
      status: 'failed' as const,
      error: { stage: 'install' as const, code: 'WORKER_RESTARTED', message: 'x' },
    }
    await stage({ ...s, check: { ...s.check!, ready: false, run } })
    expect(panel('The run here stopped at Frozen install')?.textContent).toContain(
      'A restart stopped this while it was running',
    )
    expect(btn('Run again')).not.toBeNull()
  })

  it('agent quiet: the agent row warns and its chat is the primary action', async () => {
    const openScreen = vi.fn()
    await stage(B('state-stalled'), {}, { openScreen })
    expect(el.querySelector('.iii-cloud-agent [role=status][data-variant=warn]')?.textContent).toContain(
      'No progress for 10 min',
    )
    expect(btn("Open your agent's chat")?.getAttribute('data-variant')).toBe('primary')
    await click(btn("Open your agent's chat"))
    expect(openScreen).toHaveBeenCalledWith({ screen: 'chat:sess-1' })
  })

  it('agent stopped: read what it tried, ask it to finish, or check again; never a deploy', async () => {
    const openScreen = vi.fn()
    const h = await stage(B('state-stopped'), marker, { openScreen })
    const p = panel("Your agent couldn't fix 1 problem")
    expect(p?.getAttribute('data-variant')).toBe('warn')
    expect(btn('Read what it tried')?.getAttribute('data-variant')).toBe('primary')
    await click(btn('Read what it tried'))
    expect(openScreen).toHaveBeenCalledWith({ screen: 'chat:sess-1' })
    await click(btn('Check again'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
    await click(btn('Ask your agent to finish'))
    expect(handoffs(h)[0]).toMatchObject({ payload: { kind: 'ready', check_id: 'chk-d3' } })
    expect(btn('Review and deploy')).toBeNull()
    expect(btn('Fix with your agent')).toBeNull()
  })

  it('busy: Follow #N is the only way on and Review stays locked', async () => {
    const h = await stage(B('state-busy'))
    expect(panel('Deployment #6 to dev is already running')?.textContent).toContain(
      'It started from the console 2m ago. Starting yours now would replace it.',
    )
    expect(locked()).toBe(true)
    expect(text()).toContain('Unlocks when deployment #6 is done.')
    expect(text()).not.toContain('Deploy mine after it')
    await click(btn('Follow #6'))
    expect(h.go).toHaveBeenCalledWith({ at: 'deploy', env: DEV.id, deploy: 'dep-dev-6' })
  })

  it('O1: a console override is a diff in words; "Use my file\'s" records the choice and never locks Review', async () => {
    const h = await stage(B('state-override'))
    const card = panel("web: your file replaces the console's at deploy")
    expect(card?.getAttribute('data-variant')).toBe('info')
    const lines = [...(card?.querySelectorAll('.iii-cloud-cs-diff > span') ?? [])].map((x) => x.textContent)
    expect(lines).toEqual(['console: serves web on 3000 · team only', 'your file: serves site on 8080 · team only'])
    expect(btn('Review the setting')).toBeNull()
    // The card leads: its choice first; Review stays open beside it.
    expect(btn("Use my file's")?.getAttribute('data-variant')).toBe('primary')
    expect(btn("Keep the console's")?.getAttribute('data-variant')).toBe('secondary')
    expect(btn('Review and deploy')?.getAttribute('data-variant')).toBe('secondary')
    expect(locked()).toBe(false)
    await click(btn("Use my file's"))
    expect(h.calls.filter((c) => c.id === 'iii-cloud::override::choose').map((c) => c.payload)).toEqual([
      { environment_id: DEV.id, setting: 'web', choice: 'file' },
    ])
    expect(h.go).not.toHaveBeenCalled()
  })

  it('O1: a chosen override collapses to one faint line under "Worth a look"; Review leads again', async () => {
    await stage(B('state-override-chosen'))
    expect(panel("web: your file replaces the console's at deploy")).toBeUndefined()
    expect(rowsOf('Worth a look').at(-1)).toBe('At deploy, web comes from your file.')
    expect(marksOf('Worth a look').at(-1)).toBe('worth a look')
    expect([...(region('Worth a look')?.querySelectorAll('li[data-faint]') ?? [])].map((x) => x.textContent)).toEqual([
      'At deploy, web comes from your file.',
    ])
    expect(btn('Review and deploy')?.getAttribute('data-variant')).toBe('primary')
  })

  it('O1: a file that opens it to the public stays a warn ("Make it public"); "Keep the console\'s" asks the agent to change the file', async () => {
    const h = await stage(B('setup-config-override'), marker, { scenario: 'setup' })
    const card = panel("web: your file replaces the console's at deploy")
    expect(card?.getAttribute('data-variant')).toBe('warn')
    const lines = [...(card?.querySelectorAll('.iii-cloud-cs-diff > span') ?? [])].map((x) => x.textContent)
    expect(lines).toEqual([
      'console: serves web on 3000 · team only',
      'your file: serves web on 3000 · public',
      'Your file makes it public.',
    ])
    expect(btn("Use my file's")).toBeNull()
    expect(btn('Make it public')?.getAttribute('data-variant')).toBe('primary')
    await click(btn("Keep the console's"))
    await flush()
    expect(handoffs(h)[0]).toMatchObject({ payload: { kind: 'ready', check_id: 'chk-rc1' } })
    expect(String(lastCall(handoffs(h)).payload?.text).split('\n')[0]).toBe(
      `Make api/worker-compose-prod.yaml's x-iii-cloud.web match test's console value ${JSON.stringify(webOverride.console)}, or drop it from the block, so the deploy keeps the console's.`,
    )
    expect(h.calls.filter((c) => c.id === 'iii-cloud::override::choose').map((c) => c.payload)).toEqual([
      { environment_id: TEST_ENV.id, setting: 'web', choice: 'console' },
    ])
  })

  it('branch moved: bring it into the check (a hand-off); Review locked until then', async () => {
    const h = await stage(B('state-moved'), marker)
    expect(panel("The branch dev follows has a commit your check doesn't")?.textContent).toContain(
      'dev is at 1f22ab0 now; the check ran on top of 86c49e1.',
    )
    expect(locked()).toBe(true)
    await click(btn('Bring it into the check'))
    expect(handoffs(h)[0]).toMatchObject({ payload: { kind: 'ready', check_id: 'chk-d4' } })
    expect(handoffs(h)[0]).toMatchObject({
      payload: { text: expect.stringMatching(/^dev moved to 1f22ab0: bring it into the check\.\n/) },
    })
  })

  it('signed out: Sign in again connects through the browser', async () => {
    const h = await stage(B('state-signedout'))
    const p = panel('Your iii cloud sign-in expired')
    expect(p?.getAttribute('data-variant')).toBe('alert')
    expect(p?.textContent).toContain('The run here finished and passed.')
    await click(btn('Sign in again'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::connect', payload: { method: 'browser' } })
  })

  it('out of date: check the new head from the checkout, or deploy the commit that passed without it', async () => {
    const h = await stage(B('state-outdated'), { 'iii-cloud::project::use': {}, 'iii-cloud::check': {} })
    expect(panel('billing moved after the check')?.textContent).toContain(
      '9c04e7f passed, but billing now ends at 2be71d0. Only a commit that passed can be deployed.',
    )
    expect(locked()).toBe(true)
    expect(text()).toContain('Check 2be71d0 first, or deploy 9c04e7f without it.')
    await click(btn('Check 2be71d0'))
    await flush()
    const sent = h.calls.filter((c) => c.id !== 'iii-cloud::status')
    expect(sent).toEqual([
      {
        id: 'iii-cloud::project::use',
        payload: { path: '/home/you/workspaces/registry', environment_id: DEV.id, mode: 'worktree', from: 'checkout' },
      },
      { id: 'iii-cloud::check', payload: { run: true, push: true } },
    ])
    await click(btn('Deploy 9c04e7f without it'))
    expect(h.go).toHaveBeenCalledWith({ at: 'deploy', env: DEV.id })
  })

  /** The worktree took `dev` (the environment's branch) at `taken`; the folder is on main at `head`; dev is at `devTip`. */
  const fromDev = (taken: string | null, devTip: string, head: string, commits: CommitLine[] = []): Status => {
    const s = B('state-outdated')
    const p = s.project!
    const dev: BranchRef = {
      name: 'dev',
      remote: false,
      sha: devTip,
      committed_at: s.check!.checked_at,
      ahead: 0,
      is_env_branch: true,
    }
    return {
      ...s,
      check: { ...s.check!, commits },
      project: {
        ...p,
        checkout: { ...p.checkout, branch: 'main', head, branches: [dev] },
        workspace: { ...p.workspace!, source: 'dev', source_sha: taken },
      },
    }
  }
  const sha = (seven: string) => seven.padEnd(40, '0')

  it("from the environment's branch: the person's main moving is not the check's source moving", async () => {
    await stage(fromDev(sha('86c49e1'), sha('86c49e1'), sha('2be71d0')))
    expect(text()).not.toContain('moved after the check')
    expect(locked()).toBe(false)
  })

  it('from a branch that moved: check its new tip from the same source, bringing changes as before', async () => {
    const brought = [{ sha: sha('4b8f001'), message: 'Uncommitted changes from main' }]
    const h = await stage(fromDev(sha('86c49e1'), sha('1f22ab0'), sha('86c49e1'), brought), {
      'iii-cloud::project::use': {},
      'iii-cloud::check': {},
    })
    expect(panel('dev moved after the check')?.textContent).toContain(
      '9c04e7f passed, but dev now ends at 1f22ab0. Only a commit that passed can be deployed.',
    )
    await click(btn('Check 1f22ab0'))
    await flush()
    expect(h.calls.find((c) => c.id === 'iii-cloud::project::use')?.payload).toEqual({
      path: '/home/you/workspaces/registry',
      environment_id: DEV.id,
      mode: 'worktree',
      from: 'checkout',
      source: 'dev',
      bring_changes: true,
    })
  })

  it('an older worker (no source_sha) shows no "moved" notice', async () => {
    await stage(fromDev(null, sha('1f22ab0'), sha('2be71d0')))
    expect(text()).not.toContain('moved after the check')
  })

  it('out of date and not ready: no deploy without it; a merge conflict asks the agent to merge in its copy', async () => {
    const s = { ...B('state-outdated'), check: { ...passed, ready: false, problems: [rights] } }
    const message =
      'The commits of feat/x conflict with dev: your agent can merge feat/x in the worktree /w and resolve them. Conflicting files: README.md, api/main.ts.'
    const conflict = Object.assign(new Error(message), { code: 'MERGE_CONFLICT' })
    const h = await stage(s, {
      'iii-cloud::project::use': conflict,
      'iii-cloud::handoff': { id: 'h1', marker: 'iii-cloud hand-off h1', delivered: false, reused: false },
    })
    expect(btn('Deploy 9c04e7f without it')).toBeNull()
    await click(btn('Check 2be71d0'))
    await flush()
    const p = panel(
      "Your commits conflict with the environment's branch, so they can't come in without resolving them.",
    )
    expect(p?.getAttribute('data-variant')).toBe('warn')
    expect(p?.textContent).toContain('Your agent can merge them in its copy and resolve the conflicts')
    expect(h.calls.some((c) => c.id === 'iii-cloud::check')).toBe(false)
    await click(btn('Ask your agent to bring your commits in'))
    expect(handoffs(h)[0]).toMatchObject({
      payload: {
        kind: 'ready',
        text: expect.stringMatching(
          new RegExp(
            `^Merge ${s.project!.checkout.branch} into the worktree and resolve the conflicts in README\\.md, api/main\\.ts, then check again\\.\\n`,
          ),
        ),
      },
    })
  })
})

describe('Start my agent if it finds problems', () => {
  const ended = (record: object) =>
    ({ kind: 'check', id: 'chk-d1', terminal: true, summary: '', record }) as ChangeEvent
  const box = () => el.querySelector('.iii-cloud-cs-leave input[type=checkbox]') as HTMLInputElement

  it('checked by default: hands off once when the check it watched ends not ready', async () => {
    const h = await stage(B('deploy-checking'), marker)
    expect(box().checked).toBe(true)
    await fire(h, ended(found))
    await flush()
    expect(handoffs(h)).toHaveLength(1)
    expect(handoffs(h)[0]).toMatchObject({ payload: { kind: 'ready', check_id: 'chk-d1' } })
    await fire(h, ended({ ...found }))
    await flush()
    expect(handoffs(h)).toHaveLength(1)
  })

  it('never with an agent active, never unchecked, never for a check the agent asked for', async () => {
    const busy = await stage(
      { ...B('deploy-checking'), agent: { ...B('deploy-fixing').agent!, kind: 'import' } },
      marker,
    )
    await fire(busy, ended(found))
    await flush()
    expect(handoffs(busy)).toHaveLength(0)
    const off = await stage(B('deploy-checking'), marker)
    await click(box())
    expect(box().checked).toBe(false)
    await fire(off, ended(found))
    await flush()
    expect(handoffs(off)).toHaveLength(0)
    const agents = await stage(B('deploy-checking'), marker)
    await fire(agents, ended({ ...found, handoff_id: 'ho-x' }))
    await flush()
    expect(handoffs(agents)).toHaveLength(0)
  })
})

describe('an earlier check (checkId)', () => {
  it('shows status.check as given, read-only: parts, found list and logs, and nothing to act on', async () => {
    const record = { ...B('deploy-passed'), check: found, agent: null, handoff_brief: null }
    const h = await stage(record, {}, { checkId: 'chk-d1' })
    expect(h.calls.some((c) => c.id === 'iii-cloud::check::get')).toBe(false)
    expect(title()).toBe('3 things to fix in e41a7c3')
    expect(rowsOf('It runs here like the cloud')).toHaveLength(7)
    expect(rowsOf('What the check found')).toHaveLength(2)
    expect(region('What the check found')?.querySelector('details')?.textContent).toContain('permission denied')
    // Only the sections' own folds: nothing acts on the record.
    expect([...el.querySelectorAll('button')].every((b) => b.hasAttribute('aria-expanded'))).toBe(true)
    expect(btn('Set values')).toBeNull()
    expect(el.querySelector('input')).toBeNull()
    expect(region('Checks so far')).toBeNull()
    expect(region('Needs you')).toBeNull()
  })
})

describe('the agent, alive', () => {
  it('working: state, elapsed, now, last step, steps; Open the chat; Stop asks first', async () => {
    const openScreen = vi.fn()
    const h = await stage(agentAt('working'), { 'iii-cloud::handoff::stop': {} }, { openScreen })
    const row = el.querySelector('.iii-cloud-agent')?.textContent ?? ''
    expect(row).toContain('Working')
    expect(row).toContain('17 min')
    expect(row).toContain('now: Upgrading iii-sdk to 0.27.0')
    expect(row).toContain('last step just now · 14 steps')
    expect(row).not.toContain('now ago')
    expect(btn('Fix with your agent')).toBeNull()
    await click(btn("Open your agent's chat"))
    expect(openScreen).toHaveBeenCalledWith({ screen: 'chat:sess-1' })
    // D6: the chat is secondary and Stop is a ghost that asks first.
    expect(btn("Open your agent's chat")?.getAttribute('data-variant')).toBe('secondary')
    expect(btn('Stop the agent')?.getAttribute('data-variant')).toBe('ghost')
    await click(btn('Stop the agent'))
    const dialog = el.querySelector('[role=alertdialog]')
    expect(dialog?.querySelector('b')?.textContent).toBe('Stop your agent?')
    expect(dialog?.textContent).toContain('Its commits so far stay on iii-cloud/ready-production.')
    expect(h.calls.some((c) => c.id === 'iii-cloud::handoff::stop')).toBe(false)
    await click(byText('[role=alertdialog] button', 'Stop the agent'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::handoff::stop', payload: { id: 'ho-1' } })
  })

  it('without a chat to open, the chat button is not offered', async () => {
    await stage(agentAt('working'))
    expect(btn("Open your agent's chat")).toBeNull()
  })

  it('waiting for you shows the question with the chat as primary; stalled warns', async () => {
    await stage(agentAt('waiting_for_you'), {}, { openScreen: vi.fn() })
    expect(text()).toContain('Waiting for you')
    expect(text()).toContain('Keep path:// for http and storage')
    expect(btn("Answer in your agent's chat")?.getAttribute('data-variant')).toBe('primary')
    await stage(agentAt('stalled'), {}, { openScreen: vi.fn() })
    expect(el.querySelector('.iii-cloud-agent [role=status][data-variant=warn]')?.textContent).toContain(
      'No progress for 4 min',
    )
  })

  it('waiting on a wake it armed: a neutral dot, what it waits for, the chat as secondary, no question', async () => {
    const s = agentAt('waiting')
    await stage({ ...s, agent: { ...s.agent!, waiting_for: 'the run here to end' } }, {}, { openScreen: vi.fn() })
    expect(el.querySelector('.iii-cloud-agent-line')?.textContent).toBe('Waiting · 17 min · for the run here to end')
    expect(el.querySelector('.iii-cloud-agent-mark [data-dot]')?.getAttribute('data-dot')).toBe('ghost')
    expect(text()).not.toContain('Waiting for you')
    expect(text()).not.toContain('now:')
    expect(el.querySelector('.iii-cloud-agent-q')).toBeNull()
    expect(btn("Answer in your agent's chat")).toBeNull()
    expect(btn("Open your agent's chat")?.getAttribute('data-variant')).toBe('secondary')
    expect(btn('Stop the agent')).not.toBeNull()
  })

  it('done and stopped have no Stop', async () => {
    await stage({ ...agentAt('done'), check: ready })
    expect(btn('Stop the agent')).toBeNull()
    expect(text()).not.toContain('now:')
    const s = agentAt('working')
    await stage({ ...s, agent: { ...s.agent!, state: 'stopped' } })
    expect(btn('Stop the agent')).toBeNull()
  })

  it('"N of M fixed" counts what is gone from the newest check; open rows before fixed ones', async () => {
    await stage(agentAt('waiting_for_you'))
    expect(text()).toContain('4 problems: 1 needs you · your agent is on 1 · 2 fixed')
    expect(rowsOf('Your agent is fixing')).toHaveLength(1)
    expect(rowsOf('Your agent is fixing')[0]).toContain('iii-sdk 0.24.4')
    // Fixed from both lists, in their own section.
    const fixed = rowsOf('Fixed')
    expect(fixed.some((r) => r.includes('Lock missing'))).toBe(true)
    expect(fixed.some((r) => r.includes('http and storage point at folders'))).toBe(true)
    // The board draws no "can take a while" tag.
    expect(text()).not.toContain('can take a while')
    // The two warnings are worth a look, a section of their own, folded: they won't block the deploy.
    expect(rowsOf('Worth a look').map((r) => r.slice(0, 14))).toEqual(['Migrations run', 'No healthcheck'])
    expect(noteOf('Worth a look')).toBe("won't block the deploy")
    expect(head('Worth a look')?.getAttribute('aria-expanded')).toBe('false')
    expect(rowsOf('Your agent is fixing').some((r) => r.includes('Runs with bun'))).toBe(false)
    expect(rowsOf('Needs you')).toHaveLength(1)
    expect(rowsOf('Needs you')[0]).toContain('Runs with bun')
    expect(marksOf('Needs you')).toEqual(['problem'])
  })

  it('P4: every open row of "Your agent is fixing" spins while it works, and the note under now:', async () => {
    await stage(agentAt('working'))
    const on = region('Your agent is fixing')?.querySelectorAll('[aria-label="Your agent is working on this"]')
    expect(on).toHaveLength(rowsOf('Your agent is fixing').length)
    expect(rowsOf('Your agent is fixing')[0]).toContain('iii-sdk 0.24.4')
    expect(noteOf('Your agent is fixing')).toBeUndefined()
    expect(text()).toContain('Bumping the sdk and regenerating the lock')
    // Spinners only where it works: nothing of the person's spins.
    expect(region('Needs you')?.querySelector('[aria-label="Your agent is working on this"]')).toBeNull()
  })

  it('P4: working with nothing of its own listed: "your agent is checking the run here"', async () => {
    const s = agentAt('working')
    const problems = s.check!.problems.filter((x) => x.owner !== 'agent')
    await stage({ ...s, check: { ...s.check!, problems } })
    expect(el.querySelector('.iii-cloud-cs-sub')?.textContent).toMatch(/· your agent is checking the run here ·/)
    expect(region('Your agent is fixing')).toBeNull()
  })

  it('P4: "last step 2 min ago", never "just now ago"', async () => {
    const s = agentAt('working')
    await stage({ ...s, agent: { ...s.agent!, last_activity_at: new Date(Date.now() - 125_000).toISOString() } })
    expect(el.querySelector('.iii-cloud-agent')?.textContent).toContain('last step 2 min ago')
  })

  it('aria-live is on the state word only, not the ticking timer', async () => {
    await stage(agentAt('working'))
    expect(el.querySelector('.iii-cloud-agent[aria-live]')).toBeNull()
    expect(el.querySelector('.iii-cloud-agent-head strong')?.getAttribute('aria-live')).toBe('polite')
  })

  it('the chat opens through openConversation (no second pane) when the host has it', async () => {
    const openConversation = vi.fn()
    const openScreen = vi.fn()
    await stage(agentAt('working'), {}, { chat: { compose: vi.fn(), openConversation }, openScreen })
    await click(btn("Open your agent's chat"))
    expect(openConversation).toHaveBeenCalledWith('sess-1')
    expect(openScreen).not.toHaveBeenCalled()
  })

  it('"Starting" shows no minutes before a minute', async () => {
    const s = agentAt('working')
    const fresh = { ...s.agent!, state: 'starting' as const, started_at: new Date(Date.now() - 20_000).toISOString() }
    await stage({ ...s, agent: fresh })
    expect(el.querySelector('.iii-cloud-agent-head')?.textContent).toBe('Starting')
    expect(text()).toContain('14 steps')
  })

  it('an agent event is applied live: the row appears, then the stopped panel replaces it', async () => {
    const h = await stage(base)
    const agentEvent = (record: object, terminal = false) =>
      fire(h, { kind: 'agent', id: 'ho-1', terminal, summary: '', record } as ChangeEvent)
    expect(el.querySelector('.iii-cloud-agent')).toBeNull()
    await agentEvent(agentAt('working').agent!)
    expect(el.querySelector('.iii-cloud-agent-head')?.textContent).toContain('Working')
    await agentEvent({ ...agentAt('working').agent!, state: 'done' }, true)
    expect(el.querySelector('.iii-cloud-agent')).toBeNull()
    expect(panel("Your agent couldn't fix 4 problems")).toBeTruthy()
  })
})

describe('one hand-off at a time', () => {
  it('a hand-off registers first and leads the prompt with its marker', async () => {
    const h = await stage(base, marker)
    await click(btn('Fix with your agent'))
    expect(lastCall(h.calls)).toEqual({
      id: 'iii-cloud::handoff',
      payload: {
        check_id: 'chk-blocked',
        kind: 'ready',
        text: `${handoffMessage(base)}\nDetails: iii-cloud::status · skill iii://iii-cloud`,
      },
    })
    expect(h.compose.mock.calls[0][0].text.split('\n')[0]).toBe('[iii-cloud handoff ho-9]')
  })

  it('a double click launches once and the button is off while it is in flight', async () => {
    let release: (v: unknown) => void = () => undefined
    const gate = new Promise((r) => {
      release = r
    })
    const compose = vi.fn()
    const h = await stage(base, { 'iii-cloud::handoff': () => gate }, { chat: { compose } })
    const b = btn('Fix with your agent') as HTMLButtonElement
    await act(async () => {
      b.click()
      b.click()
    })
    expect(handoffs(h)).toHaveLength(1)
    expect(btn('Fix with your agent')?.hasAttribute('disabled')).toBe(true)
    await act(async () => release({ id: 'h', marker: '[m]', delivered: false }))
    await flush()
    expect(compose).toHaveBeenCalledTimes(1)
  })

  it('with the agent already on it, there is no Fix button: its chat opens and nothing launches', async () => {
    const compose = vi.fn()
    const openScreen = vi.fn()
    const h = await stage(agentAt('working'), {}, { chat: { compose }, openScreen })
    expect(btn('Fix with your agent')).toBeNull()
    await click(btn("Open your agent's chat"))
    expect(openScreen).toHaveBeenCalledWith({ screen: 'chat:sess-1' })
    expect(handoffs(h)).toHaveLength(0)
    expect(compose).not.toHaveBeenCalled()
  })

  it('handOff itself refuses a second session for the same kind', async () => {
    const compose = vi.fn()
    const h = fakeHost(agentAt('working'))
    ;(h.host as unknown as { chat: unknown }).chat = { compose }
    await handOff(h.host, agentAt('working'), 'ready')
    expect(compose).not.toHaveBeenCalled()
    expect(handoffs(h)).toHaveLength(0)
  })

  it('a request with its own text reaches the worker even while the agent is on that kind; no new chat', async () => {
    const compose = vi.fn()
    const openScreen = vi.fn()
    const h = fakeHost(agentAt('working'), {
      'iii-cloud::handoff': { id: 'ho-1', marker: '[m]', reused: true, delivered: true, session_id: 'sess-1' },
    })
    ;(h.host as unknown as { chat: unknown }).chat = { compose }
    ;(h.host as unknown as { panels: Record<string, unknown> }).panels.openScreen = openScreen
    await handOff(h.host, agentAt('working'), 'ready', undefined, undefined, 'main moved to 1f22ab0: bring it in.')
    expect(handoffs(h)[0]).toMatchObject({
      payload: { text: expect.stringMatching(/^main moved to 1f22ab0: bring it in\.\n/) },
    })
    expect(openScreen).toHaveBeenCalledWith({ screen: 'chat:sess-1' })
    expect(compose).not.toHaveBeenCalled()
  })

  it('while the agent is mid-run on another kind, a request is sent to it, not into a new chat', async () => {
    const s = agentAt('working')
    const compose = vi.fn()
    const h = await stage({ ...s, agent: { ...s.agent!, kind: 'pull_request' } }, marker, { chat: { compose } })
    await click(btn('Send to your agent'))
    expect(handoffs(h)).toHaveLength(1)
    expect(compose).not.toHaveBeenCalled()
  })
})

describe('delivery of a hand-off', () => {
  it('delivered: no new chat, the agent chat opens and the person is told', async () => {
    const compose = vi.fn()
    const openScreen = vi.fn()
    await stage(
      base,
      { 'iii-cloud::handoff': { id: 'h', marker: '[m]', delivered: true, session_id: 'sess-7' } },
      { chat: { compose }, openScreen },
    )
    await click(btn('Fix with your agent'))
    expect(compose).not.toHaveBeenCalled()
    expect(openScreen).toHaveBeenCalledWith({ screen: 'chat:sess-7' })
    expect(text()).toContain("Sent to your agent's chat")
  })

  it('a chat that cannot be opened is a visible error with Try again, never silent', async () => {
    const compose = vi.fn(() => 'unavailable')
    await stage(base, { 'iii-cloud::handoff': { id: 'h', marker: '[m]', delivered: false } }, { chat: { compose } })
    await click(btn('Fix with your agent'))
    expect(text()).toContain("Couldn't open a chat for your agent")
    compose.mockReturnValue('composed' as never)
    await click(btn('Try again'))
    expect(compose).toHaveBeenCalledTimes(2)
    expect(text()).not.toContain("Couldn't open a chat")
  })

  it('a reused hand-off never launches a chat: it opens the session, or waits', async () => {
    const compose = vi.fn()
    const openConversation = vi.fn()
    await stage(
      base,
      { 'iii-cloud::handoff': { id: 'h', marker: '[m]', reused: true, session_id: 'sess-3' } },
      { chat: { compose, openConversation } },
    )
    await click(btn('Fix with your agent'))
    expect(compose).not.toHaveBeenCalled()
    expect(openConversation).toHaveBeenCalledWith('sess-3')
    clearAsked()
    const compose2 = vi.fn()
    await stage(
      base,
      { 'iii-cloud::handoff': { id: 'h', marker: '[m]', reused: true } },
      { chat: { compose: compose2 } },
    )
    await click(btn('Fix with your agent'))
    expect(compose2).not.toHaveBeenCalled()
    expect(text()).toContain('Sent to your agent. Waiting for it to pick this up.')
  })
})

describe('hand-offs', () => {
  const mkHost = (chat: Record<string, unknown>) => {
    const h = fakeHost(base, { 'iii-cloud::handoff': { id: 'ho-1', marker: '[iii-cloud handoff ho-1]' } })
    ;(h.host as unknown as { chat: unknown }).chat = chat
    return h
  }
  const POINTER = 'Details: iii-cloud::status · skill iii://iii-cloud'
  const WT = '~/workspaces/registry-iii-cloud'

  it('the person says one plain sentence; the details live in status and the skill', () => {
    const s = base
    expect(handoffMessage(s)).toBe(
      'Make registry · production ready for iii cloud: fix the 6 problems from the latest check.',
    )
    expect(fixMessage(failedDeploy, s)).toBe('The deploy 5f81a09 of registry · production failed at Workers. Fix it.')
    expect(handoffMessage(s)).not.toMatch(/iii-cloud::|iii:\/\//)
  })

  it('(a) startChat with a folder: the worktree and the cloud-ready profile', async () => {
    const startChat = vi.fn(async (_r: unknown) => 'sess')
    const h = mkHost({ compose: vi.fn(), startChat, startChatWorkingDir: true })
    await handOff(h.host, base, 'ready')
    const sent = startChat.mock.calls[0][0] as { text: string; agent: string; workingDir: string }
    expect(sent.agent).toBe('cloud-ready')
    expect(sent.workingDir).toBe(WT)
    expect(sent.text.split('\n')).toEqual(['[iii-cloud handoff ho-1]', handoffMessage(base), POINTER])
  })

  it('(b) a draft chat, moved into the worktree by the click, then sent', async () => {
    const compose = vi.fn()
    const openDraft = vi.fn(() => 'draft-1')
    const requestWorkingDirectoryChange = vi.fn(() => true)
    const h = mkHost({ compose, openDraft, requestWorkingDirectoryChange })
    await handOff(h.host, base, 'ready')
    expect(openDraft).toHaveBeenCalledWith({ text: '', title: handoffMessage(base) }, { show: true })
    expect(requestWorkingDirectoryChange).toHaveBeenCalledWith({ sessionId: 'draft-1', path: WT })
    expect(compose.mock.calls[0][0]).toMatchObject({ sessionId: 'draft-1', submit: true })
  })

  it('(b) a refused folder still sends, and tells the person to confirm it in the chat', async () => {
    const compose = vi.fn()
    await stage(
      base,
      { 'iii-cloud::handoff': { id: 'ho-1', marker: '[m]' } },
      { chat: { compose, openDraft: () => 'd2', requestWorkingDirectoryChange: () => false } },
    )
    await click(btn('Fix with your agent'))
    expect(compose).toHaveBeenCalled()
    expect(text()).toContain('Confirm the folder in the chat when asked.')
  })

  it('(c) otherwise the chat in front of the person', async () => {
    const compose = vi.fn()
    const h = mkHost({ compose })
    await handOff(h.host, base, 'ready')
    expect(compose.mock.calls[0][0]).toMatchObject({ submit: true, beside: 'ext:iii-cloud' })
    expect(compose.mock.calls[0][0].sessionId).toBeUndefined()
  })

  it('checkout mode works in the checkout folder', async () => {
    const s = base
    const co = { ...s, project: { ...s.project!, workspace: { ...s.project!.workspace!, mode: 'checkout' as const } } }
    const startChat = vi.fn(async (_r: unknown) => 's')
    const h = mkHost({ compose: vi.fn(), startChat, startChatWorkingDir: true })
    await handOff(h.host, co, 'ready')
    expect((startChat.mock.calls[0][0] as { workingDir: string }).workingDir).toBe('/home/you/workspaces/registry')
  })
})

describe('the redesign (CHECKSTAGE-PLAN, release-control · test while the agent fixes)', () => {
  const live = () => stage(B('setup-fixing-live'), {}, { scenario: 'setup' })
  const group = (name: string, title: string) =>
    [...(region(name)?.querySelectorAll('li.iii-cloud-cs-group') ?? [])].find((g) =>
      g.querySelector('.iii-cloud-cs-label')?.textContent?.startsWith(title),
    )
  const chips = (g: Element | undefined) => [...(g?.querySelectorAll('[data-chip]') ?? [])].map((c) => c.textContent)
  const lines = (g: Element | undefined) =>
    [...(g?.querySelectorAll('.iii-cloud-cs-each > li') ?? [])].map((l) => l.textContent)
  const writes = (h: { calls: { id: string }[] }) =>
    h.calls.filter((c) => c.id === 'iii-cloud::api::write' || c.id.startsWith('iii-cloud::secret'))

  it('D2: the 21 empty variables are one neutral row, "21 optional names …", with 21 names; open, NAME · file:line each', async () => {
    await live()
    const vars = group('Needs you', '21 optional names have no value in test; they default to empty')
    expect(vars).toBeTruthy()
    // Optional (`${X:-}`): no warning mark, nothing said twice under the title.
    expect(vars?.querySelector('[role=img]')?.getAttribute('aria-label')).toBe('optional')
    expect(vars?.querySelector('.iii-cloud-cs-detail')).toBeNull()
    expect(chips(vars)).toEqual(RC_VARIABLES)
    expect(vars?.querySelector('.iii-cloud-cs-loc')?.textContent).toBe(
      'api/worker-compose-prod.yaml:81–101 · containers.worker-release-control.environment',
    )
    expect(text()).not.toContain('A variable defaults to empty')
    expect(text()).not.toContain('See api/worker-compose-prod.yaml')
    // The board's toggle: "Show each" under the row, not the title.
    const trigger = vars?.querySelector('button.iii-cloud-cs-each-toggle')
    expect(trigger?.textContent).toBe('Show each')
    expect(trigger?.getAttribute('aria-expanded')).toBe('false')
    await click(trigger)
    expect(trigger?.getAttribute('aria-expanded')).toBe('true')
    expect(trigger?.textContent).toBe('Hide each')
    const each = lines(group('Needs you', '21 optional names'))
    expect(each).toHaveLength(21)
    // Each line has its own Skip (SKIP-VALUES).
    expect(each[0]).toBe('APP_BASE_URL · api/worker-compose-prod.yaml:81Skip')
    expect(each[20]).toBe('SLACK_NOTIFICATIONS_ENABLED · api/worker-compose-prod.yaml:101Skip')
    // Open, the chips stay above the lines.
    expect(chips(group('Needs you', '21 optional names'))).toEqual(RC_VARIABLES)
  })

  it('D2: a group of names with no default keeps the strong sentence, the warning mark and the primary Set values', async () => {
    const s = B('setup-fixing-live')
    const problems = s.check!.problems.map((p) =>
      p.code === 'EMPTY_DEFAULT_VARIABLE' ? { ...p, code: 'SECRET_MISSING' } : p,
    )
    await stage({ ...s, check: { ...s.check!, problems } }, {}, { scenario: 'setup' })
    const vars = group('Needs you', '21 secrets have no value')
    expect(vars?.querySelector('[role=img]')?.getAttribute('aria-label')).toBe('problem')
    expect(vars?.hasAttribute('data-optional')).toBe(false)
    expect(byText('li.iii-cloud-cs-group button', 'Set values')?.getAttribute('data-variant')).toBe('primary')
  })

  it('D2: the same file and line under two codes shows once (the blocking one), then groups', async () => {
    await live()
    const workers = group('Your agent is fixing', '3 workers are outdated')
    expect(chips(workers)).toEqual(['http', 'cron', 'queue'])
    expect(text()).not.toContain('Queue worker outdated')
    // Lines that don't follow each other are listed, not spanned.
    expect(workers?.querySelector('.iii-cloud-cs-loc')?.textContent).toBe('api/worker-compose-prod.yaml:40, 46, 52')
    await click(workers?.querySelector('button.iii-cloud-cs-each-toggle'))
    const each = lines(group('Your agent is fixing', '3 workers'))
    expect(each).toEqual([
      'http · api/worker-compose-prod.yaml:40',
      'cron · api/worker-compose-prod.yaml:46',
      'queue · api/worker-compose-prod.yaml:52',
    ])
    // Agent rows keep where they ran, faint.
    expect(workers?.querySelector('.iii-cloud-cs-part')?.textContent).toBe('iii cloud')
  })

  it('D6: the summary counts the deduplicated list the sections show, so it adds up', async () => {
    await live()
    expect(title()).toBe('Your agent is fixing what the check found')
    expect(el.querySelector('.iii-cloud-cs-sub')?.textContent).toBe(
      '26 problems: 22 need you · your agent is on 4 · 0 fixed',
    )
    expect(el.querySelector('.iii-cloud-cs-total')?.textContent).toBe('26 problems')
    // The three header lines are one: no "N of M fixed", no app file line while the agent works.
    expect(text()).not.toMatch(/of 26 fixed/)
    expect(el.querySelector('.iii-cloud-stephead')?.textContent).not.toContain('App file')
    expect(countOf('Needs you')).toBe('22')
    expect(countOf('Your agent is fixing')).toBe('4')
    expect(countOf('Worth a look')).toBe('4')
    const you = region('Needs you')
    const items = (r: Element | null) =>
      [...(r?.querySelectorAll('li.iii-cloud-cs-item') ?? [])].reduce(
        (n, li) => n + (li.classList.contains('iii-cloud-cs-group') ? chips(li).length : 1),
        0,
      )
    expect(items(you)).toBe(22)
    // A section counts what it lists; what is only worth a look is its own section, not counted as a problem.
    expect(items(region('Your agent is fixing'))).toBe(4)
    expect(items(region('Worth a look'))).toBe(4)
    expect(region('Worth a look')?.textContent).toContain('Engine section ignored')
  })

  it('D4/O2: blocking first, then values; the workers\' config leads the warnings and the note under "Worth a look · N"', async () => {
    const h = await live()
    // What only the person gives stays up top, blocking or not.
    const you = rowsOf('Needs you')
    expect(you[0]).toMatch(/^Runs with bun/)
    expect(you[1]).toMatch(/^21 optional names/)
    expect(you).toHaveLength(2)
    // O2: the config row is a warning: first under "Worth a look", its words, its location and its ways out, never
    // Settings (no code block).
    expect(rowsOf('Worth a look')[0]).toMatch(/^The workers' config isn't in iii cloud yet/)
    const config = region('Worth a look')?.querySelector('li.iii-cloud-cs-item')
    expect(config?.querySelector('.iii-cloud-cs-detail')?.textContent).toBe(
      "Your workers read config from ./config, which isn't in the repository. In iii cloud they start with their defaults.",
    )
    expect(config?.querySelector('details')).toBeNull()
    const acts = [...(config?.querySelectorAll('.iii-cloud-cs-acts-row > *') ?? [])].map((b) => b.textContent)
    expect(acts).toEqual(['Copy ./config to iii cloud', 'Open in iii cloud', 'Skip for now'])
    expect(btn('Copy ./config to iii cloud')?.getAttribute('data-variant')).toBe('primary')
    expect(config?.querySelector('a')?.getAttribute('href')).toBe(
      'https://cloud.iii.dev/iii/release-control/test/config',
    )
    expect(btn('Open Settings')).toBeNull()
    expect(h.go).not.toHaveBeenCalled()
    // P4: "Worth a look" is its own section, folded, after what is fixed.
    const order = [...el.querySelectorAll('[role=region]')].map((r) => r.getAttribute('aria-label'))
    expect(order).toEqual(['Needs you', 'Your agent is fixing', 'Worth a look', 'Checks so far'])
    expect(head('Worth a look')?.textContent).toBe("Worth a look4won't block the deploy")
    expect(head('Worth a look')?.getAttribute('aria-expanded')).toBe('false')
    expect(region('Worth a look')?.textContent).toContain('Engine section ignored')
  })

  it("D7/P4: who is said by the section titles; agent rows keep where they ran, the person's rows drop it", async () => {
    await live()
    expect(noteOf('Needs you')).toBe('the agent never sees secrets')
    const rows = rowsOf('Your agent is fixing')
    expect(rows.some((r) => r.includes('your agent is on it'))).toBe(false)
    // Where it ran sits at the right of an agent row's title line.
    const sdk = [...(region('Your agent is fixing')?.querySelectorAll('li.iii-cloud-cs-item') ?? [])].find((r) =>
      r.textContent?.startsWith('iii-sdk'),
    )
    expect(sdk?.querySelector('.iii-cloud-cs-tline > .iii-cloud-cs-part')?.textContent).toBe('here')
    // "Needs you" rows have no "here" / "iii cloud" tag.
    const tags = [...(region('Needs you')?.querySelectorAll('.iii-cloud-cs-part') ?? [])].map((x) => x.textContent)
    expect(tags.every((t) => t === '')).toBe(true)
  })

  it('D1: what is open by default, per state; aria-expanded on every header', async () => {
    await live()
    expect(head('Needs you')?.getAttribute('aria-expanded')).toBe('true')
    expect(head('Your agent is fixing')?.getAttribute('aria-expanded')).toBe('true')
    expect(head('Worth a look')?.getAttribute('aria-expanded')).toBe('false')
    expect(head('Checks so far')?.getAttribute('aria-expanded')).toBe('false')
    expect(region('Checks so far')?.querySelector('section')?.getAttribute('aria-hidden')).toBe('true')
    // Passed: nothing open, so only "Fixed", folded.
    await stage(B('deploy-passed'))
    expect(region('What the check found')).toBeNull()
    expect(region('Needs you')).toBeNull()
    expect(head('Fixed')?.getAttribute('aria-expanded')).toBe('false')
    // Problems before the hand-off: what the agent will fix is open.
    await stage(B('deploy-problems'))
    expect(head('What the check found')?.getAttribute('aria-expanded')).toBe('true')
  })

  it('D9/P4: on a phone too, "Needs you" and what the agent is fixing start open while it works', async () => {
    await stage(B('setup-fixing-live'), {}, { scenario: 'setup', narrow: true })
    expect(head('Needs you')?.getAttribute('aria-expanded')).toBe('true')
    expect(head('Your agent is fixing')?.getAttribute('aria-expanded')).toBe('true')
    expect(head('Checks so far')?.getAttribute('aria-expanded')).toBe('false')
    // Before the hand-off, a phone opens only what needs the person.
    await stage(B('deploy-problems'), {}, { narrow: true })
    expect(head('What the check found')?.getAttribute('aria-expanded')).toBe('false')
  })

  it("D1: the person's toggles survive a re-render (a new status from the agent's events)", async () => {
    const h = await live()
    await click(head('Checks so far'))
    await click(head('Your agent is fixing'))
    await click(group('Needs you', '21 optional names')?.querySelector('button.iii-cloud-cs-each-toggle'))
    await fire(h, { kind: 'agent', id: 'ho-rc', terminal: false, summary: '', record: { ...rcAgent, steps: 12 } })
    expect(text()).toContain('12 steps')
    expect(head('Checks so far')?.getAttribute('aria-expanded')).toBe('true')
    expect(head('Your agent is fixing')?.getAttribute('aria-expanded')).toBe('false')
    expect(lines(group('Needs you', '21 optional names'))).toHaveLength(21)
  })

  it('D5 (Values): the optional group shows Set values, secondary; the dialog saves, then the stage checks again', async () => {
    const saved = (p: Record<string, unknown>) => ({
      environment_id: TEST_ENV.id,
      results: (p.items as { name: string }[]).map((x) => ({ name: x.name, ok: true })),
    })
    const h = await stage(
      B('setup-fixing-live'),
      { 'iii-cloud::values': rcValues, 'iii-cloud::values::save': saved },
      {
        scenario: 'setup',
      },
    )
    const vars = group('Needs you', '21 optional names')
    const open = byText('li.iii-cloud-cs-group button', 'Set values')
    expect(vars?.contains(open ?? null)).toBe(true)
    // Only optional names wait for a value: nothing breaks without them, so Set values is secondary.
    expect(open?.getAttribute('data-variant')).toBe('secondary')
    await click(open)
    const dialog = el.querySelector('[role=dialog]')
    expect(dialog?.querySelector('h2')?.textContent).toBe('Values for test')
    expect(dialog?.querySelector('p')?.textContent).toBe("api/worker-compose-prod.yaml reads 22 names; 21 aren't set.")
    expect(writes(h)).toEqual([])
    await click(btn('Save 12 values and check again'))
    await flush()
    expect(h.calls.filter((c) => c.id === 'iii-cloud::values::save')).toHaveLength(1)
    expect(el.querySelector('[role=dialog]')).toBeNull()
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
    expect(writes(h)).toEqual([])
  })

  it('D6: Stop is a ghost; its ConfirmDialog names the branch, and Cancel makes no call', async () => {
    const h = await live()
    expect(btn('Stop the agent')?.getAttribute('data-variant')).toBe('ghost')
    await click(btn('Stop the agent'))
    const dialog = el.querySelector('[role=alertdialog]')
    expect(dialog?.querySelector('b')?.textContent).toBe('Stop your agent?')
    expect(dialog?.textContent).toContain('Its commits so far stay on iii-cloud/ready-test.')
    await click(byText('[role=alertdialog] button', 'Cancel'))
    expect(el.querySelector('[role=alertdialog]')).toBeNull()
    expect(h.calls.some((c) => c.id === 'iii-cloud::handoff::stop')).toBe(false)
    // The agent's grey panel: state, time and now on one line; what it said; then when and how many steps.
    expect(el.querySelector('.iii-cloud-agent-line')?.textContent).toBe(`Working · 6 min · now: ${rcAgent.current}`)
    const faint = [...el.querySelectorAll('.iii-cloud-agent-text > .iii-cloud-psmall')].map((x) => x.textContent)
    expect(faint[0]).toBe('I am preparing a separate Node manifest and preserving the ECS deployment files.')
    expect(faint[1]).toBe('last step just now · 11 steps')
    // Stop sits beside the words, small (the chat button needs a chat to open: this host has none).
    expect(el.querySelector('.iii-cloud-agent-acts')?.textContent).toBe('Stop the agent')
  })

  it('D3: one location line; an excerpt with no code renders no block; the run log stays a <details>', async () => {
    const blank = {
      ...rcSdk,
      id: 'blank',
      source: 'cloud_plan' as const,
      code: 'SOME_PLAN_CODE',
      title: 'Something in the file',
      detail: 'See api/worker-compose-prod.yaml:38. (at containers.http.worker)',
      file: 'api/worker-compose-prod.yaml',
      line: 38,
      excerpt: { start_line: 38, lines: ['', '   ', 'api/worker-compose-prod.yaml:38'] },
    }
    const coded = {
      ...blank,
      id: 'coded',
      code: 'OTHER_PLAN_CODE',
      line: 60,
      excerpt: { start_line: 58, lines: ['  http:', '    worker: x'] },
    }
    const s = B('deploy-problems')
    await stage({ ...s, check: { ...s.check!, problems: [blank, coded, rights] } })
    const rows = [...(region('What the check found')?.querySelectorAll('li.iii-cloud-cs-item') ?? [])]
    expect(rows[0].querySelector('.iii-cloud-cs-loc')?.textContent).toBe(
      'api/worker-compose-prod.yaml:38 · containers.http.worker',
    )
    expect(rows[0].querySelector('.iii-cloud-cs-detail')).toBeNull()
    expect(rows[0].querySelector('details')).toBeNull()
    expect(rows[1].querySelector('details summary')?.textContent).toBe('Lines 58–59')
    expect(rows[1].querySelector('details pre')?.textContent).toBe('  http:\n    worker: x')
    expect(rows[2].querySelector('details summary')?.textContent).toBe('Last lines of the run here')
    const none = { ...s.check!, problems: [rights], run: { ...s.check!.run!, log_tail: [] } }
    await stage({ ...s, check: none })
    expect(region('What the check found')?.querySelector('details')).toBeNull()
  })

  it('D8: the footer while the agent works, the hand-off text before it; Checks so far says the last one', async () => {
    await live()
    expect(region('Needs you')?.querySelector('.iii-cloud-cs-foot')?.textContent).toBe(
      "Your agent can't do these: they need values only you have. It keeps fixing the rest.",
    )
    expect(text()).not.toContain('Your agent asks you about these')
    expect(countOf('Checks so far')).toBe('1')
    expect(noteOf('Checks so far')).toBe(`last ${clock(rcFixing.check!.checked_at)}, 30 problems`)
    const before = { ...B('setup-fixing-live'), agent: null, handoff_brief: null, checks: [] }
    await stage(before, {}, { scenario: 'setup' })
    expect(region('Needs you')?.querySelector('.iii-cloud-cs-foot')?.textContent).toBe(
      'Your agent asks you about these in its chat after you hand it over.',
    )
  })
})

describe('CHECKSTAGE-V2', () => {
  const fwd = () => [...el.querySelectorAll('.iii-cloud-stephead-fwd button')].map((b) => b.textContent)

  it('P1: the forward action is in the header, top right; nothing is repeated below', async () => {
    await stage(B('deploy-passed'))
    expect(fwd()).toEqual(['Review and deploy'])
    expect(el.querySelector('.iii-cloud-stephead-reason')?.textContent).toBe(
      'Only the commit that passed can be deployed.',
    )
    expect([...el.querySelectorAll('button')].filter((b) => b.textContent === 'Review and deploy')).toHaveLength(1)
    expect(el.querySelector('.iii-cloud-cs-acts')).toBeNull()
    await stage(B('deploy-problems'))
    expect(fwd()).toEqual(['Fix with your agent'])
    await stage(B('setup-fixing-live'), {}, { scenario: 'setup' })
    expect(fwd()).toEqual(['Review and deploy'])
    expect(btn('Review and deploy')?.hasAttribute('disabled')).toBe(true)
    // The chip sits next to the title.
    expect(el.querySelector('.iii-cloud-stephead-title [aria-live=polite]')?.textContent).toBe('Fixing')
  })

  it('P3: the run here\'s missing values are one line in "Needs you", never a row or a chip per name', async () => {
    const saved = (p: Record<string, unknown>) => ({
      environment_id: TEST_ENV.id,
      results: (p.items as { name: string }[]).map((x) => ({ name: x.name, ok: true })),
    })
    const h = await stage(
      B('setup-fixing-here'),
      { 'iii-cloud::values': rcHereValues, 'iii-cloud::values::save': saved },
      { scenario: 'setup' },
    )
    expect(text()).not.toContain('has no value here')
    const you = region('Needs you')
    const line = [...(you?.querySelectorAll('li.iii-cloud-cs-item') ?? [])].at(-1)
    expect(line?.querySelector('.iii-cloud-cs-label')?.textContent).toBe(
      "2 values the run here needs aren't on this machine",
    )
    expect(line?.querySelector('[data-chip]')).toBeNull()
    for (const name of RC_HERE_UNSET) expect(line?.textContent).not.toContain(name)
    expect(you?.querySelectorAll('li.iii-cloud-cs-item')).toHaveLength(3)
    // Each counts: 22 + 2 need you (the workers' config is only worth a look, O2).
    expect(countOf('Needs you')).toBe('24')
    expect(el.querySelector('.iii-cloud-cs-sub')?.textContent).toMatch(/: 24 need you ·/)
    // Set values opens only those names, each "Not set", needed to run here.
    await click(line?.querySelector('button') ?? null)
    const dialog = el.querySelector('[role=dialog]')
    expect(dialog?.querySelector('p')?.textContent).toBe('The run here needs these 2 names on this machine.')
    const names = [...(dialog?.querySelectorAll('li.iii-cloud-val-row') ?? [])].map((r) => r.getAttribute('data-name'))
    expect(names.sort()).toEqual([...RC_HERE_UNSET].sort())
    for (const r of dialog?.querySelectorAll('li.iii-cloud-val-row') ?? []) {
      expect(r.textContent).toContain('needed to run here')
    }
    await type(dialog?.querySelector('input[aria-label="Value of SESSION_SECRET"]') as HTMLInputElement, 'w')
    await click(btn('Save 1 value and check again'))
    await flush()
    expect(h.calls.filter((c) => c.id === 'iii-cloud::values::save').map((c) => c.payload)).toEqual([
      expect.objectContaining({ items: [expect.objectContaining({ name: 'SESSION_SECRET', from: 'typed' })] }),
    ])
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
  })

  it('P3: a cloud value problem keeps its grouped row; the other Set values opens every name', async () => {
    await stage(B('setup-fixing-here'), { 'iii-cloud::values': rcHereValues }, { scenario: 'setup' })
    const group = [...(region('Needs you')?.querySelectorAll('li.iii-cloud-cs-group') ?? [])][0]
    expect(group?.querySelector('.iii-cloud-cs-label')?.textContent).toMatch(/^21 optional names/)
    await click(group?.querySelector('button') ?? null)
    expect(el.querySelectorAll('[role=dialog] li.iii-cloud-val-row').length).toBeGreaterThan(2)
  })
})

describe('SKIP-VALUES', () => {
  const skips = (h: { calls: { id: string; payload?: unknown }[] }) =>
    h.calls.filter((c) => c.id === 'iii-cloud::values::skip').map((c) => c.payload)
  const group = (name: string, title: string) =>
    [...(region(name)?.querySelectorAll('li.iii-cloud-cs-group') ?? [])].find((g) =>
      g.querySelector('.iii-cloud-cs-label')?.textContent?.startsWith(title),
    )
  const skipped = { environment_id: TEST_ENV.id, names: RC_SKIPPED }

  it('"Skip for now" on a group skips all its names, then checks again', async () => {
    const h = await stage(B('setup-fixing-live'), { 'iii-cloud::values::skip': skipped }, { scenario: 'setup' })
    const vars = group('Needs you', '21 optional names')
    const skip = [...(vars?.querySelectorAll('button') ?? [])].find((b) => b.textContent === 'Skip for now')
    expect(skip?.getAttribute('data-variant')).toBe('ghost')
    await click(skip ?? null)
    expect(skips(h)).toEqual([{ environment_id: TEST_ENV.id, names: RC_VARIABLES, skip: true }])
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
  })

  it('"Show each": every line has its own Skip', async () => {
    const h = await stage(B('setup-fixing-live'), { 'iii-cloud::values::skip': skipped }, { scenario: 'setup' })
    await click(group('Needs you', '21 optional names')?.querySelector('button.iii-cloud-cs-each-toggle') ?? null)
    await click(el.querySelector('[aria-label="Skip LOG_LEVEL"]'))
    expect(skips(h)).toEqual([{ environment_id: TEST_ENV.id, names: ['LOG_LEVEL'], skip: true }])
  })

  it('skipped names leave "Needs you" for a folded "Skipped · N" before "Worth a look"; nothing counts them; Undo', async () => {
    const h = await stage(
      B('setup-fixing-skipped'),
      { 'iii-cloud::values::skip': { ...skipped, names: [] } },
      {
        scenario: 'setup',
      },
    )
    expect(group('Needs you', '19 optional names')?.querySelector('.iii-cloud-cs-loc')?.textContent).toBe(
      'api/worker-compose-prod.yaml:81–94, 96, 98–101 · containers.worker-release-control.environment',
    )
    expect(countOf('Needs you')).toBe('20')
    expect(el.querySelector('.iii-cloud-cs-sub')?.textContent).toBe(
      '24 problems: 20 need you · your agent is on 4 · 0 fixed · 2 skipped',
    )
    const order = [...el.querySelectorAll('[role=region]')].map((r) => r.getAttribute('aria-label'))
    expect(order).toEqual(['Needs you', 'Your agent is fixing', 'Skipped', 'Worth a look', 'Checks so far'])
    expect(countOf('Skipped')).toBe('2')
    expect(head('Skipped')?.getAttribute('aria-expanded')).toBe('false')
    await click(head('Skipped'))
    const row = group('Skipped', '2 names have no value')
    expect([...(row?.querySelectorAll('[data-chip]') ?? [])].map((c) => c.textContent)).toEqual(RC_SKIPPED)
    expect(region('Skipped')?.querySelector('.iii-cloud-cs-foot')?.textContent).toBe(
      'Skipped for now. Set them in Values whenever you want.',
    )
    // No Set values and no Skip here: Undo brings them all back; each line has its own.
    const acts = [...(row?.querySelectorAll('.iii-cloud-cs-acts-row > button') ?? [])].map((b) => b.textContent)
    expect(acts).toEqual(['Undo', 'Show each'])
    await click([...(row?.querySelectorAll('button') ?? [])].find((b) => b.textContent === 'Undo') ?? null)
    expect(skips(h)).toEqual([{ environment_id: TEST_ENV.id, names: RC_SKIPPED, skip: false }])
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
    await click(row?.querySelector('button.iii-cloud-cs-each-toggle') ?? null)
    await click(el.querySelector('[aria-label="Undo skipping SENTRY_DSN"]'))
    expect(skips(h).at(-1)).toEqual({ environment_id: TEST_ENV.id, names: ['SENTRY_DSN'], skip: false })
  })

  it('a skipped SECRET_MISSING stays in "Needs you": iii cloud needs a value; "Leave it empty in iii cloud" saves "" and checks', async () => {
    const saved = { environment_id: TEST_ENV.id, results: RC_SKIPPED.map((name) => ({ name, ok: true })) }
    const h = await stage(B('setup-fixing-secret-skipped'), { 'iii-cloud::values::save': saved }, { scenario: 'setup' })
    expect(region('Skipped')).toBeNull()
    expect(el.querySelector('.iii-cloud-cs-sub')?.textContent).not.toContain('skipped')
    const left = group('Needs you', '2 secrets have no value')
    expect(left?.textContent).toContain('iii cloud needs a value to deploy, even an empty one.')
    const acts = [...(left?.querySelectorAll('.iii-cloud-cs-acts-row > button') ?? [])]
    expect(acts.map((b) => b.textContent)).toEqual(['Leave it empty in iii cloud', 'Set values', 'Show each'])
    expect(acts[0].getAttribute('data-variant')).toBe('primary')
    // The rest of the secrets keep Skip for now.
    expect(group('Needs you', '19 secrets have no value')?.textContent).toContain('Skip for now')
    await click(acts[0])
    expect(h.calls.filter((c) => c.id === 'iii-cloud::values::save').map((c) => c.payload)).toEqual([
      {
        environment_id: TEST_ENV.id,
        manifest: 'api/worker-compose-prod.yaml',
        items: RC_SKIPPED.map((name) => ({ name, from: 'leave_empty' })),
      },
    ])
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
  })

  it('the run-here line skips its names too', async () => {
    const h = await stage(B('setup-fixing-here'), { 'iii-cloud::values::skip': skipped }, { scenario: 'setup' })
    const line = [...(region('Needs you')?.querySelectorAll('li.iii-cloud-cs-item') ?? [])].at(-1)
    await click([...(line?.querySelectorAll('button') ?? [])].find((b) => b.textContent === 'Skip for now') ?? null)
    expect(skips(h)).toEqual([{ environment_id: TEST_ENV.id, names: RC_HERE_UNSET, skip: true }])
  })
})

describe("CONFIG-AND-OVERRIDES (O2): the workers' config, resolved in place", () => {
  const imports = (h: { calls: { id: string; payload?: unknown }[] }) =>
    h.calls.filter((c) => c.id === 'iii-cloud::config::import').map((c) => c.payload)
  const dialog = () => el.querySelector('[role=dialog]')
  const setup = { scenario: 'setup' } as const
  /** The live preview, with one secret-looking key the worker left out. */
  const withSecret = {
    ...configPreview,
    workers: configPreview.workers.map((w) => (w.worker === 'cron' ? { ...w, left_out: ['webhook_token'] } : w)),
  }

  it('Copy ./config previews the dry run (file → worker, its fields, what is left out or set by iii cloud), applies its version, then checks again', async () => {
    const h = await stage(
      B('setup-config-override'),
      {
        'iii-cloud::config::import': (p: Record<string, unknown>) =>
          p.dry_run ? withSecret : { ...configPreview, applied: true, version: 4 },
      },
      setup,
    )
    // A warning: it leads "Worth a look", never "Needs you".
    expect(rowsOf('Needs you').some((r) => r.includes("workers' config"))).toBe(false)
    expect(rowsOf('Worth a look')[0]).toMatch(/^The workers' config isn't in iii cloud yet/)
    await click(btn('Copy ./config to iii cloud'))
    expect(imports(h)).toEqual([{ environment_id: TEST_ENV.id, dry_run: true }])
    expect(dialog()?.querySelector('h2')?.textContent).toBe('Copy ./config to iii cloud')
    expect(dialog()?.textContent).toContain("What test's workers get from api/config on this machine.")
    const blocks = [...(dialog()?.querySelectorAll('.iii-cloud-ci-list > li') ?? [])]
    expect(blocks.map((b) => b.querySelector('.iii-cloud-ci-head')?.textContent)).toEqual([
      'api/config/cron.yaml→cron',
      'api/config/http.yaml→http',
      'api/config/queue.yaml→queue',
    ])
    expect(blocks[1].querySelector('pre')?.textContent).toBe(
      [
        'default_timeout = 30000',
        'concurrency_request_limit = 1024',
        'body_limit = 10mb',
        'cors.allowed_origins = ["https://release.iii.dev"]',
        'cors.allowed_methods = ["GET","POST","PUT","DELETE","OPTIONS"]',
      ].join('\n'),
    )
    expect(blocks[2].querySelector('pre')?.textContent?.split('\n')).toHaveLength(12)
    // Left out and set by iii cloud are separate faint lines; keys only, never a value.
    expect(blocks[0].querySelector('.iii-cloud-ci-out')?.textContent).toBe(
      `webhook_token left out: set it as a secret and write ${['$', '{NAME}'].join('')} in iii cloud`,
    )
    expect(dialog()?.textContent).not.toContain('webhook_token =')
    expect(blocks[1].querySelector('.iii-cloud-ci-out')?.textContent).toBe('Set by iii cloud: host, port')
    expect(dialog()?.querySelector('[aria-label="Not copied"]')?.textContent).toBe(
      'api/config/iii-observability.yaml not copied: no worker iii cloud runs reads it',
    )
    expect(h.calls.some((c) => c.id === 'iii-cloud::check')).toBe(false)
    await click(btn("Copy 3 workers' config to test"))
    await flush()
    expect(imports(h)).toEqual([
      { environment_id: TEST_ENV.id, dry_run: true },
      { environment_id: TEST_ENV.id, dry_run: false, expected_version: 3 },
    ])
    expect(dialog()).toBeNull()
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
  })

  it("a config changed since the preview (CONFIG_CHANGED) says so in the worker's words and reads the preview again", async () => {
    const changed = Object.assign(new Error("Someone changed test's config; review again."), { code: 'CONFIG_CHANGED' })
    const h = await stage(
      B('setup-config-override'),
      {
        'iii-cloud::config::import': (p: Record<string, unknown>) => {
          if (!p.dry_run) throw changed
          return configPreview
        },
      },
      setup,
    )
    await click(btn('Copy ./config to iii cloud'))
    await click(btn("Copy 3 workers' config to test"))
    await flush()
    expect(panel("Someone changed test's config; review again.")?.getAttribute('data-variant')).toBe('warn')
    expect(imports(h).map((p) => (p as { dry_run: boolean }).dry_run)).toEqual([true, false, true])
    expect(dialog()?.querySelectorAll('.iii-cloud-ci-list > li')).toHaveLength(3)
    expect(h.calls.some((c) => c.id === 'iii-cloud::check')).toBe(false)
  })

  it("a refusal (no config folder, role, edits pending in the console) keeps the worker's sentence; pending edits wait, with the way to them", async () => {
    const missing = Object.assign(new Error("./config isn't there next to the compose file: nothing to copy."), {
      code: 'CONFIG_FOLDER_MISSING',
    })
    await stage(B('setup-config-override'), { 'iii-cloud::config::import': missing }, setup)
    await click(btn('Copy ./config to iii cloud'))
    expect(panel("./config isn't there next to the compose file: nothing to copy.")?.getAttribute('data-variant')).toBe(
      'alert',
    )
    expect(btn('Copy to test')?.hasAttribute('disabled')).toBe(true)
    await stage(
      B('setup-config-override'),
      { 'iii-cloud::config::import': { ...configPreview, draft_changed: true } },
      setup,
    )
    await click(btn('Copy ./config to iii cloud'))
    const pending = panel("test has config edits in iii cloud that aren't applied yet")
    expect(pending?.querySelector('a')?.getAttribute('href')).toBe(
      'https://cloud.iii.dev/iii/release-control/test/config',
    )
    expect(btn("Copy 3 workers' config to test")?.hasAttribute('disabled')).toBe(true)
  })

  it('Skip for now (`problem::skip`) moves it under "Skipped" (info) with Undo; each checks again', async () => {
    let codes: string[] = []
    const s = B('setup-config-override')
    const h = await stage(
      () => ({ ...s, skipped_problems: codes }),
      {
        'iii-cloud::problem::skip': (p: Record<string, unknown>) => {
          codes = p.skip ? ['LOCAL_CONFIG_NOT_SET'] : []
          return { environment_id: TEST_ENV.id, codes }
        },
      },
      setup,
    )
    await click(byText('[aria-label="Worth a look"] button', 'Skip for now'))
    await flush()
    const skips = () => h.calls.filter((c) => c.id === 'iii-cloud::problem::skip').map((c) => c.payload)
    expect(skips()).toEqual([{ environment_id: TEST_ENV.id, code: 'LOCAL_CONFIG_NOT_SET', skip: true }])
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
    expect(region('Worth a look')?.textContent).not.toContain("The workers' config isn't in iii cloud yet")
    expect(rowsOf('Skipped')[0]).toMatch(/^The workers' config isn't in iii cloud yet/)
    expect(marksOf('Skipped')).toEqual(['skipped'])
    expect(region('Skipped')?.querySelector('.iii-cloud-cs-foot')?.textContent).toBe(
      "Skipped for now. Copy the workers' config from Settings whenever you want.",
    )
    const acts = [...(region('Skipped')?.querySelectorAll('.iii-cloud-cs-acts-row > *') ?? [])].map(
      (b) => b.textContent,
    )
    expect(acts).toEqual(['Undo'])
    await click(byText('[aria-label="Skipped"] button', 'Undo'))
    await flush()
    expect(skips().at(-1)).toEqual({ environment_id: TEST_ENV.id, code: 'LOCAL_CONFIG_NOT_SET', skip: false })
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
    expect(region('Skipped')).toBeNull()
    expect(rowsOf('Worth a look')[0]).toMatch(/^The workers' config isn't in iii cloud yet/)
  })

  it('checked again after the skip: the check marks it (info); it stays under "Skipped", never "Fixed"', async () => {
    await stage(B('setup-config-skipped'), {}, setup)
    expect(rowsOf('Skipped')[0]).toMatch(/^The workers' config isn't in iii cloud yet/)
    expect(region('Fixed')).toBeNull()
    expect(region('Worth a look')?.textContent).not.toContain("The workers' config isn't in iii cloud yet")
    // Undone before the next check: a warning again.
    await stage({ ...B('setup-config-skipped'), skipped_problems: [] }, {}, setup)
    expect(region('Skipped')).toBeNull()
    expect(rowsOf('Worth a look')[0]).toMatch(/^The workers' config isn't in iii cloud yet/)
  })

  it('"Your agent can\'t do these…" only under blocking items', async () => {
    await stage(B('setup-fixing-live'), {}, setup)
    expect(region('Needs you')?.querySelector('.iii-cloud-cs-foot')?.textContent).toBe(
      "Your agent can't do these: they need values only you have. It keeps fixing the rest.",
    )
    const s = B('setup-fixing-live')
    const check = s.check ? { ...s.check, problems: s.check.problems.filter((p) => p.severity !== 'block') } : null
    await stage({ ...s, check }, {}, setup)
    expect(region('Needs you')).toBeTruthy()
    expect(region('Needs you')?.querySelector('.iii-cloud-cs-foot')).toBeNull()
  })
})

describe('only warnings left: the header counts what blocks, and a run here never run is what remains', () => {
  it('registry · dev: iii cloud accepts it, two warnings, a note, four skipped names, no run here: "Run it here"', async () => {
    const h = await stage(B('deploy-run-pending'))
    expect(title()).toBe('iii cloud accepts this commit; run it here to finish the check')
    expect(chip()).toBe('Run here pending')
    expect(el.querySelector('.iii-cloud-cs-sub')?.textContent).toBe(
      'The run here starts your app like iii cloud does: install, build, release, then your workers.',
    )
    expect(text()).not.toMatch(/things? to fix/)
    expect(btn('Check again')).toBeNull()
    // The warnings and the note are only worth a look; nothing needs the person.
    expect(region('Needs you')).toBeNull()
    const worth = rowsOf('Worth a look')
    expect(worth).toHaveLength(3)
    expect(worth[0]).toMatch(/^The workers' config isn't in iii cloud yet/)
    expect(worth[1]).toMatch(/^SQLite is replaced by Postgres/)
    expect(worth[2]).toMatch(/^Engine section ignored/)
    const go = btn('Run it here')
    expect(go?.getAttribute('data-variant')).toBe('primary')
    await click(go)
    expect(lastCall(h.calls)).toMatchObject({ id: 'iii-cloud::check', payload: { run: true } })
  })

  it('warnings never count as things to fix; blocking items do', async () => {
    const s = B('deploy-run-pending')
    const c = s.check!
    await stage({
      ...s,
      check: { ...c, plan: { ...c.plan, status: 'blocked', blocking: 1 }, problems: [...c.problems, stripe] },
    })
    expect(title()).toBe('1 thing to fix before dev can get this')
  })

  it('a run here that failed keeps its stop and the usual title', async () => {
    const s = B('deploy-run-pending')
    const failed = {
      ...baseRun,
      check_id: 'chk-d6',
      status: 'failed' as const,
      error: { stage: 'release' as const, code: 'RELEASE_FAILED', message: 'db:migrate exited with 1.' },
    }
    await stage({ ...s, check: { ...s.check!, run: failed } })
    expect(title()).toBe('Not ready for dev yet')
    expect(chip()).toBe('Not ready')
    expect(panel('The run here stopped at Release command')).toBeTruthy()
    expect(btn('Run it here')).toBeNull()
  })
})
