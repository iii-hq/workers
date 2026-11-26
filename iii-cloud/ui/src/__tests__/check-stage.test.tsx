// The check on the Deploy screen (CheckStage): its header says the next thing to do with the lists' own numbers; You
// has the action in each item (a secret's field, a decision's two ways, the check's words for the rest); Your agent
// starts by itself and keeps Stop on screen; Details is one closed fold; Ready says what the deploy does.
import type { Host } from '@iii-dev/console-ui'
import { act } from 'react'
import { describe, expect, it, type Mock, vi } from 'vitest'
import { LaunchNote } from '../AgentRow'
import { CheckStage } from '../CheckStage'
import { boards } from '../fixtures'
import { agent, base, pr, ready } from '../fixtures/base'
import { found, passed, rcFixing } from '../fixtures/check'
import { devValues } from '../fixtures/values'
import { useStatus } from '../hub'
import type { Route } from '../route'
import type { ChangeEvent, Status } from '../types'
import { fakeHost } from './host'
import { btn, click, el, flush, lastCall, root, text, type, usePage } from './util'

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
    expect(text()).toContain('Unlocks when a check passes')
    expect(h.calls.filter((c) => c.id !== 'iii-cloud::status')).toEqual([])
    await click(btn('Check'))
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check', payload: { run: true, push: true } })
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
    const you = Number(/(\d+) things? needs? you/.exec(t)?.[1] ?? 0)
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
      const n = Number(/^(\d+) thing/.exec(title() ?? '')?.[1])
      await stage(first(), read())
      expect(fillRow()?.textContent).toContain(
        '2 of 4 names your compose file reads can be filled from .env and .env.local.',
      )
      expect(fillRow()?.textContent).toContain('Review and save')
      expect(title()).toMatch(new RegExp(`^${n + 1} things? `))
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

  it("the agent's fixes going back to the person's branch is stated as a fact", async () => {
    // Newest first: two commits by the agent, then the last one the folder gave (the workspace's from_head).
    const commits = [
      { sha: 'a'.repeat(40), message: 'Pin search' },
      { sha: 'b'.repeat(40), message: 'Fix it' },
      { sha: 'e41a7c3'.padEnd(40, '0'), message: 'Search worker' },
    ]
    await stage({ ...dev, check: { ...passed, commits } })
    expect(region('What this deploy does')?.textContent).toContain(
      "After the deploy, billing gets your agent's 2 fixes",
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

describe('the words', () => {
  it('no old word on any state of the check', async () => {
    for (const id of ['deploy-checking', 'deploy-problems', 'deploy-fixing', 'deploy-passed'] as const) {
      await stage(B(id))
      expect(text()).not.toMatch(/app file|Needs you|Worth a look|the run here|Superseded|Cancelled/)
    }
  })
})
