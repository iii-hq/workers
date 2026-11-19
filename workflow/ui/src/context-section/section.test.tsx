// @vitest-environment jsdom

import type { ContextSectionSummary, Host } from '@iii-dev/console-ui'
import uiClasses from '@iii-dev/console-ui/ui-classes'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { BATCH, KIT, KIT_NEXT, LIVE, T } from '../lib/__tests__/run-fixtures'
import { registerWorkflowSection, WorkflowSection } from './index'
import { resetWorkflowReads, WORKFLOW_RUN_LIMIT } from './use-runs'
;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

// The console serves StatusDot through its import map; the package throws
// when loaded directly. The stand-in shows its tone and pulse.
vi.mock('@iii-dev/console-ui', () => ({
  StatusDot: ({ tone, pulse }: { tone?: string; pulse?: boolean }) => (
    <span data-dot={tone} data-pulse={pulse ? '' : undefined} />
  ),
}))

const mocks = {
  call: vi.fn(),
  setSummary: vi.fn(),
  openConversation: vi.fn(),
  selectConversation: vi.fn(),
}

/** `workflow::status` replies by run id; a run left out reads as gone. */
let RUNS: Record<string, unknown> = {}
/** The runs `workflow::list` names for the chat, newest first. */
let LIST: string[] = []
/** The `title` `workflow::list` gives a run, by run id. */
let TITLES: Record<string, string> = {}

function respond(fn: string, payload: Record<string, unknown> = {}) {
  if (fn === 'workflow::list')
    return Promise.resolve({
      runs: LIST.map((run_id, i) => ({
        run_id,
        status: 'running',
        title: TITLES[run_id],
        created_at: 1000 - i,
        updated_at: 1000 - i,
      })),
    })
  if (fn === 'workflow::status') return Promise.resolve(RUNS[String(payload.run_id)] ?? null)
  return Promise.reject(new Error(`unexpected ${fn}`))
}

let host: Pick<Host, 'iii' | 'chat'>
let root: Root | null = null
let container: HTMLElement | null = null

async function render({ open = true, active = true }: { open?: boolean; active?: boolean } = {}) {
  if (!container) {
    container = document.createElement('div')
    // The panel's body around a section.
    container.className = uiClasses.tree
    document.body.appendChild(container)
    root = createRoot(container)
  }
  await act(async () => {
    root?.render(
      <WorkflowSection
        host={host}
        sessionId="main"
        active={active}
        open={open}
        isStreaming={false}
        setSummary={mocks.setSummary}
      />,
    )
  })
  await flush()
  return container
}

/** The list, then the reads it names, then their replies. */
async function flush() {
  for (let i = 0; i < 4; i += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
  }
}

const summary = (): ContextSectionSummary => mocks.setSummary.mock.calls.at(-1)?.[0] ?? {}
const inSection = (selector: string) => container?.querySelector(selector) as HTMLButtonElement | null
const statusCalls = () => mocks.call.mock.calls.filter(([fn]) => fn === 'workflow::status')
const phaseHeader = (phase: string) => inSection(`button[aria-label^="${phase} phase,"]`)
/** A run's own row: its tooltip ends with the run id. */
const runRow = (runId: string) => inSection(`button[aria-expanded][title*=" · ${runId}"]`)
const byText = (text: string) =>
  [...(container?.querySelectorAll('*') ?? [])].find(
    (el) => el.childElementCount === 0 && el.textContent?.trim() === text,
  ) as HTMLElement | undefined

const FAILED = {
  ...BATCH,
  run_id: 'r_fail',
  status: 'failed',
  nodes: { 'batch-check': 'failed', 'batch-work': 'done' },
  node_errors: { 'batch-check': 'verifier exited with code 2' },
  result_error: 'node batch-check failed',
  node_details: {
    ...BATCH.node_details,
    'batch-check': {
      ...BATCH.node_details['batch-check'],
      state: 'failed',
    },
  },
}

beforeEach(() => {
  resetWorkflowReads()
  for (const mock of Object.values(mocks)) mock.mockReset()
  mocks.call.mockImplementation(respond)
  host = {
    iii: { call: mocks.call } as unknown as Host['iii'],
    chat: {
      openConversation: mocks.openConversation,
      selectConversation: mocks.selectConversation,
    } as unknown as Host['chat'],
  }
  RUNS = {}
  LIST = []
  TITLES = {}
})

afterEach(() => {
  act(() => root?.unmount())
  container?.remove()
  root = null
  container = null
})

describe('the Workflow section', () => {
  it('registers where the console has the slot', () => {
    const remove = vi.fn()
    const registerContextSection = vi.fn(() => remove)
    const withSlot = { chat: { registerContextSection } } as unknown as Host
    expect(registerWorkflowSection(withSlot)).toBe(remove)
    expect(registerContextSection).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'workflow', title: 'Workflow', order: 450 }),
    )
    expect(registerWorkflowSection({ chat: {} } as unknown as Host)).toBeUndefined()
    expect(registerWorkflowSection({} as unknown as Host)).toBeUndefined()
  })

  it('stays out of a chat that started no run', async () => {
    const view = await render()
    expect(summary()).toMatchObject({ count: 0, empty: true })
    expect(view.textContent).toBe('')
    expect(mocks.call).toHaveBeenCalledWith('workflow::list', { caller_session_id: 'main' }, expect.anything())
    expect(statusCalls()).toEqual([])
  })

  it('makes no call while out of sight', async () => {
    LIST = ['r_3f03']
    RUNS.r_3f03 = BATCH
    await render({ active: false })
    expect(mocks.call).not.toHaveBeenCalled()
  })

  it('shows a run by phase, its nodes in the order they ran', async () => {
    LIST = ['r_3f03']
    RUNS.r_3f03 = BATCH
    await render()
    expect(mocks.call).toHaveBeenCalledWith('workflow::status', { run_id: 'r_3f03', details: true }, expect.anything())
    const text = container?.textContent ?? ''
    // Named after what its node labels share.
    expect(text).toContain('KAN-5 batch r1w1')
    expect(text).toContain('test → check · 292k tokens · 1m 51s')
    expect(summary()).toMatchObject({ count: 1, empty: false, brief: '1 completed', live: undefined })
    // A phase of one node is that node's row: nothing to fold.
    expect(
      ['test', 'check'].map((phase) => [
        phaseHeader(phase)?.getAttribute('aria-expanded'),
        phaseHeader(phase)?.getAttribute('data-workflow-phase'),
      ]),
    ).toEqual([
      [null, 'done'],
      [null, 'done'],
    ])
    expect(text).toContain('tests · codex/gpt-6-luna · 256k tokens')
    expect(text).toContain('1m 25s')
    expect(text).toContain('25.9s')
    expect(inSection('button[aria-label^="test phase, Open tests"]')?.title).toContain('workflow-impl')
    const labels = [...(container?.querySelectorAll('button[aria-label]') ?? [])].map((button) =>
      button.getAttribute('aria-label'),
    )
    expect(labels.indexOf('test phase, Open tests in a new panel (done)')).toBeLessThan(
      labels.indexOf('check phase, Open check in a new panel (done)'),
    )
  })

  it('names a run with what the agent said it is for', async () => {
    const TITLE = 'Writing and checking batch regression tests'
    LIST = ['r_3f03']
    TITLES.r_3f03 = TITLE
    RUNS.r_3f03 = BATCH
    await render()
    const text = container?.textContent ?? ''
    expect(text).toContain(TITLE)
    // The title no longer names the nodes, so their rows keep their whole label.
    expect(text).toContain('KAN-5 batch r1w1 tests · codex/gpt-6-luna · 256k tokens')
    expect(phaseHeader('test')?.getAttribute('aria-label')).toBe(
      'test phase, Open KAN-5 batch r1w1 tests in a new panel (done)',
    )
  })

  it("opens a node's session from its row", async () => {
    LIST = ['r_3f03']
    RUNS.r_3f03 = BATCH
    await render()
    const test = phaseHeader('test')
    expect(test?.getAttribute('aria-label')).toBe('test phase, Open tests in a new panel (done)')
    act(() => test?.click())
    expect(mocks.openConversation).toHaveBeenCalledWith('wf_r_3f03_batch-work@r0')
    expect(mocks.selectConversation).not.toHaveBeenCalled()
  })

  it('switches to the session on a console without a pane to open it in', async () => {
    LIST = ['r_3f03']
    RUNS.r_3f03 = BATCH
    host = { ...host, chat: { selectConversation: mocks.selectConversation } as unknown as Host['chat'] }
    await render()
    act(() => phaseHeader('test')?.click())
    expect(mocks.selectConversation).toHaveBeenCalledWith('wf_r_3f03_batch-work@r0')
  })

  it('marks a live run, its loop passes and its waiting nodes', async () => {
    LIST = ['r_live']
    RUNS.r_live = LIVE
    await render()
    expect(summary().live).toBe('1 running')
    // The phase with a failure and the running one open; waiting ones fold.
    expect(
      ['work', 'check', 'plan', 'map', 'critic'].map((phase) => phaseHeader(phase)?.getAttribute('aria-expanded')),
    ).toEqual(['true', 'true', null, null, null])
    const suffixes = [...(container?.querySelectorAll('span.wf-ctx-suffix') ?? [])].map((span) => span.textContent)
    expect(suffixes).toContain('#it1')
    // A waiting phase of one node named like it reads as one line.
    const plan = phaseHeader('plan')
    expect(plan?.disabled).toBe(true)
    expect(plan?.getAttribute('aria-label')).toBe('plan phase, plan (pending)')
    expect(plan?.textContent).toBe('planpending')
    // A node without a phase sits beside the headers, one level in.
    expect(inSection('button[aria-label="a_report (pending)"]')?.getAttribute('style')).toContain(
      '--iii-ui-tree-depth: 1',
    )
  })

  it("lists a run's nodes under its phases, the current one open", async () => {
    LIST = ['r_kit']
    RUNS.r_kit = KIT
    await render()
    const read = (phase: string) => {
      const header = phaseHeader(phase)
      return [
        header?.getAttribute('data-workflow-phase'),
        header?.getAttribute('aria-expanded'),
        header?.textContent,
        // The current phase's name is in ink, the others quiet.
        !header?.querySelector('.wf-ctx-faint'),
      ]
    }
    expect(read('explore')).toEqual(['done', 'false', 'explore2/2', false])
    expect(read('design')).toEqual(['running', 'true', 'design1 failed · 0/2', true])
    expect(read('refute')).toEqual(['pending', 'false', 'refute0/2', false])
    expect(read('plan')).toEqual(['pending', null, 'planpending', false])
    expect(phaseHeader('design')?.getAttribute('aria-label')).toBe('design phase, 1 failed, 0 of 2 done (running)')
    // The current phase leads with a spinner; the recipe stills it under reduced motion.
    const spinner = phaseHeader('design')?.querySelector('svg.lucide-loader-circle')
    expect(spinner?.getAttribute('class')).toContain(uiClasses.spin)
    expect(phaseHeader('design')?.getAttribute('aria-current')).toBe('step')
    expect(phaseHeader('refute')?.querySelector('.lucide-loader-circle')).toBe(null)
    expect(phaseHeader('refute')?.hasAttribute('aria-current')).toBe(false)
    // Phases fold without a chevron; the run row keeps its own.
    expect(container?.querySelector('[data-workflow-phase] [class*="lucide-chevron"]')).toBeNull()
    expect(runRow('r_kit')?.querySelector('.lucide-chevron-right')).not.toBeNull()
    // Under its header a node leaves the phase out of its label, not its name.
    const minimal = inSection('button[aria-label="Open design:minimal in a new panel (running)"]')
    expect(minimal?.textContent).toContain('minimal')
    expect(minimal?.textContent).not.toContain('design:')
    expect(minimal?.getAttribute('style')).toContain('--iii-ui-tree-depth: 2')
    const robust = inSection('button[aria-label="Open design:robust in a new panel (failed)"]')
    expect(robust?.querySelector('.wf-ctx-alert')?.textContent).toBe('node timed out')
    expect(inSection('button[aria-label^="Open explore:"]')).toBeNull()
  })

  it('keeps a phase the way the person left it while the run moves on', async () => {
    LIST = ['r_kit']
    RUNS.r_kit = KIT
    await render()
    act(() => phaseHeader('explore')?.click())
    expect(phaseHeader('explore')?.getAttribute('aria-expanded')).toBe('true')
    // Folding the section and back reads the live run again.
    RUNS.r_kit = KIT_NEXT
    await render({ open: false })
    await render({ open: true })
    const state = (phase: string) => {
      const header = phaseHeader(phase)
      return [header?.getAttribute('aria-expanded'), !header?.querySelector('.wf-ctx-faint')]
    }
    expect(state('explore')).toEqual(['true', false])
    // Design settled with a failure: open to show it, no longer current.
    expect(state('design')).toEqual(['true', false])
    expect(state('refute')).toEqual(['true', true])
    expect(state('plan')).toEqual([null, false])
  })

  it("shows a stopped run's failed node apart from its cancelled ones", async () => {
    // design:robust failed (tolerated), then the stop cancelled the rest.
    LIST = ['r_stop']
    RUNS.r_stop = {
      ...KIT,
      run_id: 'r_stop',
      status: 'cancelled',
      nodes: {
        ...KIT.nodes,
        'design-minimal': 'cancelled',
        'refute-minimal': 'cancelled',
        'refute-robust': 'cancelled',
      },
      phases: {
        ...KIT.phases,
        design: { total: 2, done: 0, failed: 2, cancelled: 1, running: 0 },
        refute: { total: 2, done: 0, failed: 2, cancelled: 2, running: 0 },
      },
      node_details: {
        ...KIT.node_details,
        'design-minimal': {
          ...KIT.node_details['design-minimal'],
          state: 'cancelled',
        },
        'refute-minimal': {
          ...KIT.node_details['refute-minimal'],
          state: 'cancelled',
        },
        'refute-robust': {
          ...KIT.node_details['refute-robust'],
          state: 'cancelled',
        },
      },
    }
    await render()
    const design = phaseHeader('design')
    expect(design?.getAttribute('data-workflow-phase')).toBe('failed')
    expect(design?.getAttribute('aria-expanded')).toBe('true')
    expect(design?.textContent).toContain('1 failed · 1 cancelled · 0/2')
    expect(design?.getAttribute('aria-label')).toBe('design phase, 1 failed, 1 cancelled, 0 of 2 done (failed)')
    expect(design?.querySelector('.wf-ctx-alert')).not.toBeNull()
    // A phase the stop cancelled outright reads cancelled, folded.
    const refute = phaseHeader('refute')
    expect(refute?.getAttribute('data-workflow-phase')).toBe('cancelled')
    expect(refute?.getAttribute('aria-expanded')).toBe('false')
    expect(refute?.textContent).toContain('2 cancelled · 0/2')
    expect(refute?.querySelector('.wf-ctx-alert')).toBeNull()
    expect(refute?.querySelector('.wf-ctx-warn')).not.toBeNull()
    expect(summary()).toMatchObject({ brief: '1 cancelled', tone: undefined, live: undefined })
  })

  it('reads a phase of one running node as one row', async () => {
    LIST = ['r_kit']
    RUNS.r_kit = {
      ...KIT,
      nodes: { ...KIT.nodes, plan: 'running' },
      phases: {
        ...KIT.phases,
        plan: { total: 1, done: 0, failed: 0, running: 1 },
      },
      node_details: {
        ...KIT.node_details,
        plan: {
          state: 'running',
          label: 'plan',
          phase: 'plan',
          model: 'claude/opus-5-5',
          started_at: T + 8000,
          session_id: 'wf_r_kit_plan@r0',
        },
      },
    }
    await render()
    expect(container?.querySelectorAll('button[aria-label*="plan"]')).toHaveLength(1)
    const plan = phaseHeader('plan')
    expect(plan?.getAttribute('aria-label')).toBe('plan phase, Open plan in a new panel (running)')
    expect(plan?.querySelector('.lucide-loader-circle')).not.toBeNull()
    expect(plan?.getAttribute('aria-current')).toBe('step')
    expect(plan?.hasAttribute('aria-expanded')).toBe(false)
    // One live mark: the leading spinner, no trailing dot.
    expect(plan?.querySelector('[data-pulse]')).toBeNull()
    expect(plan?.getAttribute('style')).toContain('--iii-ui-tree-depth: 1')
    expect(plan?.textContent).toContain('claude/opus-5-5')
    act(() => plan?.click())
    expect(mocks.openConversation).toHaveBeenCalledWith('wf_r_kit_plan@r0')
  })

  it('keeps the idle dot of a phase of one running node', async () => {
    LIST = ['r_kit']
    RUNS.r_kit = {
      ...KIT,
      nodes: { ...KIT.nodes, plan: 'running' },
      phases: {
        ...KIT.phases,
        plan: { total: 1, done: 0, failed: 0, running: 1 },
      },
      node_details: {
        ...KIT.node_details,
        plan: {
          state: 'running',
          label: 'plan',
          phase: 'plan',
          started_at: Date.now() - 300_000,
          last_generate_at: Date.now() - 90_000,
          session_id: 'wf_r_kit_plan@r0',
        },
      },
    }
    await render()
    const plan = phaseHeader('plan')
    expect(plan?.querySelector('.lucide-loader-circle')).not.toBeNull()
    expect(plan?.querySelector('[data-dot="warn"]')).not.toBeNull()
    expect(plan?.textContent).toContain('idle')
  })

  it('leaves a phase whose nodes the status left out without a fold', async () => {
    const { 'explore-code': _, 'explore-options': __, ...listed } = KIT_NEXT.node_details
    LIST = ['r_kit']
    RUNS.r_kit = { ...KIT_NEXT, node_details: listed }
    await render()
    const explore = phaseHeader('explore')
    expect(explore?.textContent).toContain('2/2')
    expect(explore?.hasAttribute('aria-expanded')).toBe(false)
    expect(explore?.disabled).toBe(true)
  })

  it('shows why a run and its node failed', async () => {
    LIST = ['r_fail']
    RUNS.r_fail = FAILED
    await render()
    expect(runRow('r_fail')?.getAttribute('aria-label')).toBe('KAN-5 batch r1w1 (failed)')
    expect(runRow('r_fail')?.querySelector('.wf-ctx-alert')?.textContent).toBe('node batch-check failed')
    const node = inSection('button[aria-label="check phase, Open check in a new panel (failed)"]')
    expect(node?.title).toContain('verifier exited with code 2')
    // The leading ✗ is alert too: the error is the span.
    expect(node?.querySelector('span.wf-ctx-alert')?.textContent).toBe('verifier exited with code 2')
    expect(summary()).toMatchObject({ brief: '1 failed', tone: 'alert' })
  })

  it('folds older finished runs to one row', async () => {
    LIST = ['r_3f03', 'r_base']
    RUNS.r_3f03 = BATCH
    RUNS.r_base = {
      run_id: 'r_base',
      status: 'completed',
      created_at: 1794407000000,
      updated_at: 1794407012600,
      nodes: { baseline: 'done' },
      node_details: {
        baseline: {
          state: 'done',
          label: 'KAN-5 baseline',
          session_id: 'wf_r_base_baseline@r0',
          started_at: 1794407000000,
          finished_at: 1794407012600,
        },
      },
    }
    await render()
    const newest = inSection('button[aria-label="KAN-5 batch r1w1 (completed)"]')
    expect(newest?.getAttribute('aria-expanded')).toBe('true')
    // A run of one node is that node's row: nothing to fold, it opens the session.
    const older = inSection('button[aria-label="Open KAN-5 baseline in a new panel (completed)"]')
    expect(older?.hasAttribute('aria-expanded')).toBe(false)
    expect(older?.textContent).toContain('12.6s')
    expect(container?.textContent?.split('KAN-5 baseline')).toHaveLength(2)
    act(() => older?.click())
    expect(mocks.openConversation).toHaveBeenCalledWith('wf_r_base_baseline@r0')
    act(() => newest?.click())
    expect(newest?.getAttribute('aria-expanded')).toBe('false')
    expect(container?.textContent).not.toContain('tests · codex')
    expect(summary()).toMatchObject({ count: 2, brief: '2 completed' })
  })

  it('keeps an older run open while it is going or after it failed', async () => {
    const done = (runId: string) => ({ ...BATCH, run_id: runId })
    LIST = ['r_new', 'r_fail', 'r_live', 'r_old', 'r_done']
    RUNS.r_done = done('r_done')
    RUNS.r_old = done('r_old')
    RUNS.r_live = LIVE
    RUNS.r_fail = FAILED
    RUNS.r_new = done('r_new')
    await render()
    // Three runs show before "Show N more".
    const more = [...(container?.querySelectorAll('button') ?? [])]
      .filter((button) => button.textContent === 'Show 2 more')
      .at(-1)
    act(() => more?.click())
    expect(LIST.map((runId) => runRow(runId)?.getAttribute('aria-expanded'))).toEqual([
      'true',
      'true',
      'true',
      'false',
      'false',
    ])
    expect(summary()).toMatchObject({ count: 5, brief: '1 failed · 3 completed', tone: 'alert', live: '1 running' })
  })

  it('counts the runs and nodes it does not list', async () => {
    RUNS.r_wide = {
      ...BATCH,
      run_id: 'r_wide',
      nodes: { ...BATCH.nodes, more_a: 'done', more_b: 'done' },
    }
    LIST = ['r_wide', ...Array.from({ length: WORKFLOW_RUN_LIMIT + 1 }, (_, i) => `r_old${i}`)]
    await render()
    const text = container?.textContent ?? ''
    expect(text).toContain('2 earlier runs not listed')
    expect(text).toContain('2 more nodes not listed')
  })

  it('caps the nodes of a run', async () => {
    const uids = Array.from({ length: 12 }, (_, i) => `map#${i}`)
    LIST = ['r_map']
    RUNS.r_map = {
      run_id: 'r_map',
      status: 'completed',
      nodes: Object.fromEntries(uids.map((uid) => [uid, 'done'])),
      node_details: Object.fromEntries(
        uids.map((uid, i) => [uid, { state: 'done', session_id: `wf_${uid}`, started_at: 1000 + i }]),
      ),
    }
    await render()
    const nodeRows = () => container?.querySelectorAll('button[aria-label^="Open map #"]').length
    expect(nodeRows()).toBe(8)
    const more = [...(container?.querySelectorAll('button') ?? [])].find(
      (button) => button.textContent === 'Show 4 more',
    )
    act(() => more?.click())
    expect(nodeRows()).toBe(12)
  })

  it('caps the nodes of each phase', async () => {
    const uids = Array.from({ length: 10 }, (_, i) => `map#${i}`)
    LIST = ['r_fan']
    RUNS.r_fan = {
      run_id: 'r_fan',
      status: 'running',
      nodes: {
        ...Object.fromEntries(uids.map((uid) => [uid, 'running'])),
        check: 'pending',
        more_a: 'pending',
        more_b: 'pending',
      },
      phases: {
        map: { total: 10, done: 0, failed: 0, running: 10 },
        check: { total: 3, done: 0, failed: 0, running: 0 },
      },
      node_details: {
        ...Object.fromEntries(
          uids.map((uid, i) => [
            uid,
            {
              state: 'running',
              label: 'map:item',
              phase: 'map',
              session_id: `wf_${uid}`,
              started_at: 1000 + i,
            },
          ]),
        ),
        check: { state: 'pending', label: 'check', phase: 'check' },
      },
    }
    await render()
    const nodeRows = () => container?.querySelectorAll('button[aria-label^="Open map:item #"]').length
    expect(nodeRows()).toBe(8)
    const more = [...(container?.querySelectorAll('button') ?? [])].find(
      (button) => button.textContent === 'Show 2 more',
    )
    // "Show more" sits under its phase, before the next header.
    expect(more?.nextElementSibling).toBe(phaseHeader('check'))
    act(() => more?.click())
    expect(nodeRows()).toBe(10)
    // The nodes the status left out are counted once, at the run's end.
    const unlisted = byText('2 more nodes not listed')
    expect(unlisted?.previousElementSibling).toBe(phaseHeader('check'))
  })

  it('reads only the status of each run while folded', async () => {
    LIST = ['r_fail', 'r_3f03']
    RUNS.r_3f03 = BATCH
    RUNS.r_fail = FAILED
    await render({ open: false })
    expect(statusCalls().length).toBe(2)
    for (const [, payload] of statusCalls()) expect(payload).toMatchObject({ details: false })
    expect(summary()).toMatchObject({ count: 2, brief: '1 failed · 1 completed', tone: 'alert' })
    expect(inSection('button[aria-label^="Open "]')).toBeNull()
  })

  it('leaves out a run the worker no longer has', async () => {
    LIST = ['r_3f03']
    await render()
    expect(statusCalls().length).toBe(1)
    expect(summary()).toMatchObject({ count: 0, empty: true })
    expect(container?.textContent).toBe('')
  })
})
