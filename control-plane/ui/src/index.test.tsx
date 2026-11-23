// @vitest-environment jsdom
import { readFileSync } from 'node:fs'
import { act, type ComponentProps } from 'react'
;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Host } from '@iii-dev/console-ui'
import setup from '../page'
import { TooltipProvider, host } from './test-ui'
vi.mock('@iii-dev/console-ui', async () => (await import('./test-ui')).mockUi)
import { ControlPlane, ControlPlaneScreen } from '.'
import * as overviewHook from './useControlPlaneOverview'
import { controlPlaneFixture, controlPlaneSummaryFixture } from './fixtures'
import type { ControlPlaneSummary } from '../../src/summary'

let root: Root
let container: HTMLDivElement
beforeEach(() => {
  vi.clearAllMocks()
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})
afterEach(async () => {
  await act(() => root.unmount())
  container.remove()
})
async function render(props: Partial<ComponentProps<typeof ControlPlaneScreen>> = {}) {
  await act(() =>
    root.render(
      <TooltipProvider>
        <ControlPlaneScreen host={host} data={controlPlaneFixture} scope="all" onScopeChange={() => {}} {...props} />
      </TooltipProvider>,
    ),
  )
}
async function click(label: string) {
  const button = Array.from(container.querySelectorAll('button')).find(
    (node) => node.textContent?.trim() === label || node.childNodes[0]?.textContent?.trim() === label,
  )
  expect(button, label).toBeDefined()
  await act(() => button?.click())
}
function writer(result: ControlPlaneSummary = controlPlaneSummaryFixture) {
  return {
    ...controlPlaneFixture.summary,
    result,
    checked_at: result.checked_at,
    fresh: Date.now() - Date.parse(result.checked_at) < 30 * 60_000,
    loading: false,
    error: null,
    refresh: vi.fn(),
    now: Date.now(),
  }
}

describe('Control plane views', () => {
  it('registers the page and a settings form that picks the summary model from the chat catalog', async () => {
    const id = 'workers-qa-qa-control-plane'
    const unregisterPage = vi.fn()
    const unregisterCommands = vi.fn()
    const unregisterForm = vi.fn()
    const registerPage = vi.fn<Host['pages']['register']>(() => unregisterPage)
    const registerCommands = vi.fn<NonNullable<Host['commands']>['register']>(() => unregisterCommands)
    const registerForm = vi.fn<Host['configForms']['register']>(() => unregisterForm)
    const call = vi.fn(async (fn: string): Promise<any> =>
      fn === 'router::models::list'
        ? {
            models: [
              { id: 'claude-code/claude-sonnet-5-5', provider: 'claude-code', display_name: 'Claude Sonnet 5.5' },
              { id: 'codex/gpt-6-luna', provider: 'openai-codex', display_name: 'GPT-6-Luna (Codex)' },
            ],
          }
        : { id },
    )
    const dispose = await setup({
      ...host,
      iii: { ...host.iii, call },
      pages: { register: registerPage },
      commands: { register: registerCommands },
      configForms: { register: registerForm },
    })

    expect(call).toHaveBeenCalledWith('control-plane::configuration-id', {})
    expect(registerPage).toHaveBeenCalledWith(expect.objectContaining({
      id: 'control-plane',
      title: 'Control plane',
      configurationId: id,
    }))
    expect(registerForm).toHaveBeenCalledWith(id, expect.any(Function))
    const hook = vi.spyOn(overviewHook, 'useControlPlaneOverview').mockReturnValue({
      ...controlPlaneFixture,
      summary: { ...controlPlaneFixture.summary, error: 'set the summary model in the Control plane settings' },
    })
    try {
      const Page = registerPage.mock.calls[0][0].render
      await act(() => root.render(<Page tabId="" panelSide="left" />))
      expect(container.textContent).toContain('No summary for this project')
      await click('Open settings')
      expect(window.location.hash).toBe(`#/configuration/workers/${id}`)
    } finally {
      hook.mockRestore()
    }
    const Form = registerForm.mock.calls[0][1]
    const onChange = vi.fn()
    const value = { stall_after_min: 30, summary: { model: 'provider::model', interval_min: 10 } }
    await act(() => root.render(<Form id={id} schema={null} value={value} onChange={onChange} />))
    await act(async () => {})
    const change = async (field: string, text: string) => {
      const input = container.querySelector<HTMLInputElement>(`input[data-field="${field}"]`)!
      const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
      await act(() => {
        setValue?.call(input, text)
        input.dispatchEvent(new Event('input', { bubbles: true }))
      })
    }
    expect(call).toHaveBeenCalledWith('router::models::list', {})
    const picker = container.querySelector<HTMLSelectElement>('select[data-model-picker]')!
    expect([...picker.options].map((option) => option.value)).toEqual([
      '',
      'claude-code::claude-code/claude-sonnet-5-5',
      'openai-codex::codex/gpt-6-luna',
    ])
    const pick = async (model: string) =>
      act(() => {
        picker.value = model
        picker.dispatchEvent(new Event('change', { bubbles: true }))
      })
    await pick('claude-code::claude-code/claude-sonnet-5-5')
    expect(onChange).toHaveBeenLastCalledWith({
      ...value,
      summary: { model: 'claude-code::claude-code/claude-sonnet-5-5', interval_min: 10 },
    })
    await pick('')
    expect(onChange).toHaveBeenLastCalledWith({ ...value, summary: { interval_min: 10 } })
    await change('stall_after_min', '45')
    expect(onChange).toHaveBeenLastCalledWith({ ...value, stall_after_min: 45 })
    const [pageId, [command]] = registerCommands.mock.calls[0]
    expect(pageId).toBe('control-plane')
    expect(command).toMatchObject({ id: 'open', title: 'Open control plane' })
    command.run()
    expect(host.panels?.open).toHaveBeenCalledWith({ pageId: 'control-plane' })
    dispose()
    expect(unregisterCommands).toHaveBeenCalledOnce()
    expect(unregisterPage).toHaveBeenCalledOnce()
    expect(unregisterForm).toHaveBeenCalledOnce()
  })

  it('offers and rebinds decision chats only within a workspace tab', async () => {
    const openDecisionChat = vi.fn()
    const rebindDecisionChat = vi.fn(async () => null)
    const pageHost = { ...host, chat: { ...host.chat!, openDecisionChat, rebindDecisionChat } }
    const hook = vi.spyOn(overviewHook, 'useControlPlaneOverview').mockReturnValue(controlPlaneFixture)
    try {
      await act(() => root.render(<ControlPlane host={pageHost} tabId="" panelSide="left" />))
      expect(container.textContent).not.toContain('Ask about this')
      expect(rebindDecisionChat).not.toHaveBeenCalled()
      await act(() => root.render(<ControlPlane host={pageHost} tabId="workspace" panelSide="left" />))
      expect(container.textContent).toContain('Ask about this')
      expect(rebindDecisionChat).toHaveBeenCalled()
    } finally {
      hook.mockRestore()
    }
  })

  it('renders fresh Summary, today’s Diary and live notes as plain text with resolved links and agent attribution', async () => {
    const text = '<img src=x onerror=alert(1)> **Literal prose**'
    const summary = writer({
      ...controlPlaneSummaryFixture,
      brief: {
        ...controlPlaneSummaryFixture.brief,
        text,
        refs: ['KAN-616', 'session:dispatch-r1', 'iii-hq/iii-cloud#451', 'KAN-99999'],
      },
      diary: [
        ...controlPlaneSummaryFixture.diary,
        {
          at: new Date(Date.now() - 48 * 60 * 60_000).toISOString(),
          text: 'Yesterday’s milestone',
          refs: ['KAN-616'],
        },
      ],
    })
    await render({ summary })
    expect(container.textContent).toContain(text)
    expect(container.querySelector('img')).toBeNull()
    expect(container.textContent).toContain(controlPlaneSummaryFixture.diary[0].text)
    expect(container.textContent).not.toContain('Yesterday’s milestone')
    expect(container.textContent).toContain('KAN-616 is running the verification checks.')
    expect(container.textContent).toContain('Draft until KAN-616 finishes verification.')
    expect(container.textContent).toContain('agent ·')
    expect(container.textContent).not.toContain('KAN-99999')
    const session = [...container.querySelectorAll<HTMLButtonElement>('.control-plane-summary-refs button')].find(
      (node) => node.textContent === 'session:dispatch-r1',
    )
    await act(() => session?.click())
    expect(host.chat?.openConversation).toHaveBeenCalledWith('dispatch-r1')
    expect(container.querySelector('.control-plane-summary-refs a')?.getAttribute('href')).toBe(controlPlaneFixture.prs[0].url)
  })

  it('uses writer notes in Runs and falls back when they become stale or their live subject is removed', async () => {
    const summary = writer()
    await render({ summary, initialView: 'runs' })
    expect(container.textContent).toContain('KAN-616 is running the verification checks.')
    await render({
      summary: writer({
        ...controlPlaneSummaryFixture,
        checked_at: new Date(Date.now() - 31 * 60_000).toISOString(),
      }),
      initialView: 'runs',
    })
    expect(container.textContent).not.toContain('KAN-616 is running the verification checks.')
    expect(container.textContent).toContain('from the transcript')
    expect(container.querySelector('.control-plane-freshness .bg-warn')).not.toBeNull()
  })

  it('replaces stale Overview prose even if no writer freshness event arrives', async () => {
    const summary = writer({
      ...controlPlaneSummaryFixture,
      checked_at: new Date(Date.now() - 42 * 60_000).toISOString(),
    })
    summary.fresh = true
    await render({ summary })
    expect(container.textContent).toContain('Summary 42 min ago')
    expect(container.textContent).toContain('The summary is out of date')
    expect(container.textContent).not.toContain(controlPlaneSummaryFixture.brief.headline)
    expect(container.textContent).not.toContain(controlPlaneSummaryFixture.diary[0].text)
    expect(container.textContent).toContain('from the transcript')
    expect(container.textContent).toContain('from GitHub')
    await click('Refresh')
    expect(summary.refresh).toHaveBeenCalledOnce()
  })

  it('keeps an empty fresh Diary as a one-line state and refreshes facts and writer from the header', async () => {
    const summary = writer({ ...controlPlaneSummaryFixture, diary: [] })
    const refresh = vi.fn()
    await render({ summary, data: { ...controlPlaneFixture, refresh } })
    expect(container.textContent).toContain('No diary for this project yet')
    await act(() => container.querySelector<HTMLButtonElement>('[aria-label="Refresh Control plane"]')?.click())
    expect(summary.refresh).toHaveBeenCalledOnce()
    expect(refresh).toHaveBeenCalledOnce()
  })

  it('shows loading and unavailable one-line states while facts remain visible', async () => {
    await render({ summary: { ...writer(), result: null, loading: true } })
    expect(container.textContent).toContain('Writing the summary')
    await render({
      summary: {
        ...writer(),
        result: null,
        error: 'control-plane-writer is not installed',
      },
    })
    expect(container.textContent).toContain('The summary is unavailable')
    expect(container.textContent).toContain('from the transcript')
  })

  it('warns about summary freshness only after thirty minutes', async () => {
    await render({ summaryCheckedAt: Date.now() - 4 * 60_000 })
    expect(container.querySelector('.control-plane-freshness .bg-ok')).not.toBeNull()
    expect(container.textContent).not.toContain('The summary is out of date')
    await render({ summaryCheckedAt: Date.now() - 42 * 60_000 })
    expect(container.querySelector('.control-plane-freshness .bg-warn')).not.toBeNull()
    expect(container.textContent).toContain('The summary is out of date')
  })

  it('waits for the board before claiming that a scoped Runs view is empty', async () => {
    await render({
      initialView: 'runs',
      scope: 'assistente',
      data: {
        ...controlPlaneFixture,
        runs: [],
        loading: { ...controlPlaneFixture.loading, board: true },
      },
    })
    expect(container.querySelector('[aria-busy="true"]')).not.toBeNull()
    expect(container.textContent).not.toContain('No runs in this project yet')
    await render({
      initialView: 'runs',
      scope: 'assistente',
      data: {
        ...controlPlaneFixture,
        runs: [],
        errors: { ...controlPlaneFixture.errors, board: 'Kanban unavailable' },
      },
    })
    expect(container.textContent).toContain('Run card links could not be listed')
  })

  it('shows absent prose slots, Now items, and transcript fallbacks without a primary action', async () => {
    await render()
    expect(container.textContent).toContain('No summary for this project')
    expect(container.textContent).toContain('No diary for this project yet')
    expect(container.textContent).toContain('from the transcript')
    expect(
      Array.from(container.querySelectorAll('[data-urgency="now"] button')).filter(
        (node) => node.textContent?.trim() === 'Open',
      ),
    ).toHaveLength(2)
    expect(container.querySelector('button[data-variant="default"]')).toBeNull()
    await click('Open')
    expect(container.textContent).toContain('Or answer in your own words')
    expect(container.textContent).toContain(controlPlaneFixture.items[0].title)
  })

  it('isolates GitHub failure and uses a dash for the Open PRs tile', async () => {
    await render({
      data: {
        ...controlPlaneFixture,
        prs: [],
        errors: { ...controlPlaneFixture.errors, prs: 'unreachable' },
      },
    })
    expect(container.textContent).toContain('Pull requests are not listed')
    expect(container.textContent).toContain('Assistente · R1')
    expect(container.textContent).toContain('Board')
    const tile = Array.from(container.querySelectorAll('.control-plane-tile')).find((node) =>
      node.textContent?.includes('Open PRs'),
    )
    expect(tile?.querySelector('strong')?.textContent).toBe('—')
  })

  it('keeps available Needs you items visible when another source fails', async () => {
    await render({
      data: {
        ...controlPlaneFixture,
        errors: { ...controlPlaneFixture.errors, needsYou: 'Questions are unavailable' },
      },
    })
    expect(container.textContent).toContain('Some Needs you sources are unavailable')
    expect(container.textContent).toContain(controlPlaneFixture.items[0].title)
    const waiting = Array.from(container.querySelectorAll('.control-plane-tile')).find((node) =>
      node.textContent?.trim().startsWith('Waiting'),
    )
    expect(waiting?.querySelector('strong')?.textContent).toBe('—')
  })

  it('clears the selected question when it leaves the project scope', async () => {
    await render()
    await click('Open')
    expect(container.textContent).toContain(controlPlaneFixture.items[0].title)
    await render({ scope: 'empty', data: { ...controlPlaneFixture, items: [] } })
    expect(container.textContent).not.toContain(controlPlaneFixture.items[0].title)
  })

  it('shows a healthy wake as waiting for wake and excludes it from the Waiting tile', async () => {
    const run = controlPlaneFixture.runs[0]
    const data = {
      ...controlPlaneFixture,
      runs: [
        {
          ...run,
          state: 'waiting' as const,
          needsYou: [],
          status: { ...run.status, expects_wake: true, open_asks: 0 },
        },
      ],
    }
    await render({ data })
    expect(container.textContent).toContain('Waiting for wake')
    const tile = Array.from(container.querySelectorAll('.control-plane-tile')).find((node) =>
      node.textContent?.trim().startsWith('Waiting'),
    )
    expect(tile?.querySelector('strong')?.textContent).toBe('0')
  })

  it('labels progress bars and opens the real session and card screens', async () => {
    await render({ initialView: 'runs' })
    expect(container.querySelector('[role="progressbar"]')?.getAttribute('aria-label')).toContain(
      '11 of 14 done, 2 in progress',
    )
    expect(container.textContent).toContain('not started')
    await click('Open session')
    expect(host.chat?.openConversation).toHaveBeenCalledWith('dispatch-r1')
    await click('Open card')
    expect(host.panels?.open).toHaveBeenCalledWith({
      pageId: 'kanban-ticket',
      context: { id: 'KAN-616' },
    })
  })

  it('renders each run’s context size and uses “just now” for a new run', async () => {
    const now = Date.now()
    const runs = controlPlaneFixture.runs.slice(0, 2).map((run, index) => ({
      ...run,
      session: { ...run.session, created_at: now },
      lastProgress: now,
      context_size: [8137, 8665][index],
    }))
    await render({ initialView: 'runs', data: { ...controlPlaneFixture, runs } })
    const cards = [...container.querySelectorAll('.control-plane-run')]
    expect(cards.map((card) => card.textContent)).toEqual([
      expect.stringContaining(`Context size${(8137).toLocaleString()} tokens`),
      expect.stringContaining(`Context size${(8665).toLocaleString()} tokens`),
    ])
    expect(cards[0].textContent).toContain('started just now')
    expect(cards[0].textContent).not.toContain('just now ago')
  })

  it('shows a failed progress read without claiming the card has no subtasks', async () => {
    await render({
      initialView: 'runs',
      data: {
        ...controlPlaneFixture,
        runs: [{ ...controlPlaneFixture.runs[0], subtasks: null, progress: [] }],
      },
    })
    expect(container.textContent).toContain('Card progress unavailable')
    expect(container.textContent).not.toContain('No subtasks on this card')
  })

  it('renders Runs failure and retry while keeping the common header', async () => {
    const refresh = vi.fn()
    await render({
      initialView: 'runs',
      data: {
        ...controlPlaneFixture,
        errors: {
          ...controlPlaneFixture.errors,
          sessions: 'session-manager unavailable',
        },
        refresh,
      },
    })
    expect(container.textContent).toContain('Control plane')
    expect(container.textContent).toContain('Sessions could not be listed')
    await click('Retry')
    expect(refresh).toHaveBeenCalledOnce()
  })
})

describe('canvas overview', () => {
  it('lays the Overview out as the canvas: summary, tiles, then Needs you and Diary beside releases, runs, PRs and board', async () => {
    await render({ summary: writer() })
    const columns = [...container.querySelectorAll('.control-plane-columns > .control-plane-stack')].map((column) =>
      [...column.querySelectorAll('h2')].map((node) => node.textContent),
    )
    expect(columns).toEqual([
      ['Needs you', 'Diary'],
      ['Releases', 'In progress', 'Pull requests', 'Board'],
    ])
    expect(container.querySelectorAll('.control-plane-tile')).toHaveLength(5)
    expect(container.querySelector('details.control-plane-folded')).toBeNull()
    for (const gone of ['Direction', 'Exceptions', 'Decided for you', 'At risk', 'Target date has passed', 'target 20'])
      expect(container.textContent).not.toContain(gone)
  })

  it('lists the Now items with their count chip and folds No rush into one row', async () => {
    await render({ summary: writer() })
    const needs = [...container.querySelectorAll('.control-plane-columns h2')]
      .find((node) => node.textContent === 'Needs you')
      ?.closest('.control-plane-columns > .control-plane-stack > *')
    expect(needs?.querySelector('.control-plane-now-chip')?.textContent).toMatch(/\d+ now/)
    expect(needs?.textContent).toContain('build machine')
    expect(needs?.textContent).not.toContain('Verify timed out twice')
    expect(needs?.querySelector('.control-plane-fold-row')?.textContent).toMatch(/No rush\s*\d+/)
  })

  it('shows the decision wait median on Decided', async () => {
    await render({})
    await click('Decided')
    expect(container.textContent).toContain('Median decision wait · 90 min over the last 7 days')
  })

  it('shows the worker failure with the last valid summary age and refresh throttling', async () => {
    await render({
      summary: {
        ...writer(),
        error: 'update failed',
        fresh: false,
        throttled_until: new Date(Date.now() + 60_000).toISOString(),
      },
    })
    expect(container.textContent).toContain('The last update failed; this is the last valid summary')
    expect(container.textContent).toContain(controlPlaneSummaryFixture.brief.headline)
    expect(container.textContent).toContain('Refresh available')
    expect(container.querySelector('.control-plane-freshness .bg-warn')).not.toBeNull()
  })

  it.each([
    ['writer profile missing: apply the control plane kit', 'writer profile is not installed'],
    ['set the summary model in the Control plane settings', 'Choose the model that writes it'],
  ])('shows "%s" as the canvas no-summary panel', async (error, detail) => {
    await render({ summary: { ...writer(), error, fresh: false, unavailable: true } })
    expect(container.textContent).toContain('No summary for this project')
    expect(container.textContent).toContain(detail)
    expect(container.textContent).not.toContain(controlPlaneSummaryFixture.brief.headline)
  })

  it('hides the empty detail column at split-pane widths', () => {
    const stylesheet = new CSSStyleSheet()
    stylesheet.replaceSync(readFileSync('src/control-plane.css', 'utf8'))
    const scope = [...stylesheet.cssRules].find(
      (rule): rule is CSSStyleRule => rule instanceof CSSStyleRule && rule.selectorText === '[data-iii-ui="control-plane"]',
    )
    const stylesAt = (width: number, selector: string) => {
      const query = [...(scope?.cssRules ?? [])].find(
        (rule): rule is CSSContainerRule => rule instanceof CSSContainerRule && rule.conditionText === `(max-width: ${width}px)`,
      )
      return [...(query?.cssRules ?? [])].find(
        (rule): rule is CSSStyleRule => rule instanceof CSSStyleRule && rule.selectorText === `& ${selector}`,
      )?.style
    }
    expect(stylesAt(760, '.control-plane-inbox:not([data-detail-open]) .control-plane-decision-layout')?.display).toBe('none')
  })

  it('shows stale PR checks and omits subtree usage metrics', async () => {
    await render({ initialView: 'overview' })
    expect(container.textContent).toContain('Checked 12m ago')
    await click('Runs')
    expect(container.textContent).not.toContain('Usage')
    expect(container.textContent).not.toContain('17M')
  })
})
