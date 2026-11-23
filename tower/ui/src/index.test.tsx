// @vitest-environment jsdom
import { act, type ComponentProps } from 'react'
;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider, host } from './test-ui'
vi.mock('@iii-dev/console-ui', async () => (await import('./test-ui')).mockUi)
import { Tower, TowerScreen } from '.'
import * as overviewHook from './useTowerOverview'
import { towerFixture, towerSummaryFixture } from './fixtures'
import type { TowerSummary } from '../../src/summary'

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
async function render(props: Partial<ComponentProps<typeof TowerScreen>> = {}) {
  await act(() =>
    root.render(
      <TooltipProvider>
        <TowerScreen host={host} data={towerFixture} scope="all" onScopeChange={() => {}} {...props} />
      </TooltipProvider>,
    ),
  )
}
async function click(label: string) {
  const button = Array.from(container.querySelectorAll('button')).find((node) => node.textContent?.trim() === label)
  expect(button, label).toBeDefined()
  await act(() => button?.click())
}
function writer(result: TowerSummary = towerSummaryFixture) {
  return {
    ...towerFixture.summary,
    result,
    checked_at: result.checked_at,
    fresh: Date.now() - Date.parse(result.checked_at) < 30 * 60_000,
    loading: false,
    error: null,
    refresh: vi.fn(),
    now: Date.now(),
  }
}

describe('Control tower views', () => {
  it('offers and rebinds decision chats only within a workspace tab', async () => {
    const openDecisionChat = vi.fn()
    const rebindDecisionChat = vi.fn(async () => null)
    const pageHost = { ...host, chat: { ...host.chat!, openDecisionChat, rebindDecisionChat } }
    const hook = vi.spyOn(overviewHook, 'useTowerOverview').mockReturnValue(towerFixture)
    try {
      await act(() => root.render(<Tower host={pageHost} tabId="" panelSide="left" />))
      expect(container.textContent).not.toContain('Ask about this')
      expect(rebindDecisionChat).not.toHaveBeenCalled()
      await act(() => root.render(<Tower host={pageHost} tabId="workspace" panelSide="left" />))
      expect(container.textContent).toContain('Ask about this')
      expect(rebindDecisionChat).toHaveBeenCalled()
    } finally {
      hook.mockRestore()
    }
  })

  it('renders fresh Summary, today’s Diary and live notes as plain text with resolved links and agent attribution', async () => {
    const text = '<img src=x onerror=alert(1)> **Literal prose**'
    const summary = writer({
      ...towerSummaryFixture,
      brief: {
        ...towerSummaryFixture.brief,
        text,
        refs: ['KAN-616', 'session:dispatch-r1', 'iii-hq/iii-cloud#451', 'KAN-99999'],
      },
      diary: [
        ...towerSummaryFixture.diary,
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
    expect(container.textContent).toContain(towerSummaryFixture.diary[0].text)
    expect(container.textContent).not.toContain('Yesterday’s milestone')
    expect(container.textContent).toContain('KAN-616 is running the verification checks.')
    expect(container.textContent).toContain('Draft until KAN-616 finishes verification.')
    expect(container.textContent).toContain('agent ·')
    expect(container.textContent).not.toContain('KAN-99999')
    const session = [...container.querySelectorAll<HTMLButtonElement>('.tower-summary-refs button')].find(
      (node) => node.textContent === 'session:dispatch-r1',
    )
    await act(() => session?.click())
    expect(host.chat?.openConversation).toHaveBeenCalledWith('dispatch-r1')
    expect(container.querySelector('.tower-summary-refs a')?.getAttribute('href')).toBe(towerFixture.prs[0].url)
  })

  it('uses writer notes in Runs and falls back when they become stale or their live subject is removed', async () => {
    const summary = writer()
    await render({ summary, initialView: 'runs' })
    expect(container.textContent).toContain('KAN-616 is running the verification checks.')
    await render({
      summary: writer({
        ...towerSummaryFixture,
        checked_at: new Date(Date.now() - 31 * 60_000).toISOString(),
      }),
      initialView: 'runs',
    })
    expect(container.textContent).not.toContain('KAN-616 is running the verification checks.')
    expect(container.textContent).toContain('from the transcript')
    expect(container.querySelector('.tower-freshness .bg-warn')).not.toBeNull()
  })

  it('replaces stale Overview prose even if no writer freshness event arrives', async () => {
    const summary = writer({
      ...towerSummaryFixture,
      checked_at: new Date(Date.now() - 42 * 60_000).toISOString(),
    })
    summary.fresh = true
    await render({ summary })
    expect(container.textContent).toContain('Summary 42 min ago')
    expect(container.textContent).toContain('The summary is out of date')
    expect(container.textContent).not.toContain(towerSummaryFixture.brief.headline)
    expect(container.textContent).not.toContain(towerSummaryFixture.diary[0].text)
    expect(container.textContent).toContain('from the transcript')
    expect(container.textContent).toContain('from GitHub')
    await click('Refresh')
    expect(summary.refresh).toHaveBeenCalledOnce()
  })

  it('keeps an empty fresh Diary as a one-line state and refreshes facts and writer from the header', async () => {
    const summary = writer({ ...towerSummaryFixture, diary: [] })
    const refresh = vi.fn()
    await render({ summary, data: { ...towerFixture, refresh } })
    expect(container.textContent).toContain('No diary for this project yet')
    await act(() => container.querySelector<HTMLButtonElement>('[aria-label="Refresh Control tower"]')?.click())
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
        error: 'tower-writer is not installed',
      },
    })
    expect(container.textContent).toContain('The summary is unavailable')
    expect(container.textContent).toContain('from the transcript')
  })

  it('warns about summary freshness only after thirty minutes', async () => {
    await render({ summaryCheckedAt: Date.now() - 4 * 60_000 })
    expect(container.querySelector('.tower-freshness .bg-ok')).not.toBeNull()
    expect(container.textContent).not.toContain('The summary is out of date')
    await render({ summaryCheckedAt: Date.now() - 42 * 60_000 })
    expect(container.querySelector('.tower-freshness .bg-warn')).not.toBeNull()
    expect(container.textContent).toContain('The summary is out of date')
  })

  it('waits for the board before claiming that a scoped Runs view is empty', async () => {
    await render({
      initialView: 'runs',
      scope: 'assistente',
      data: {
        ...towerFixture,
        runs: [],
        loading: { ...towerFixture.loading, board: true },
      },
    })
    expect(container.querySelector('[aria-busy="true"]')).not.toBeNull()
    expect(container.textContent).not.toContain('No runs in this project yet')
    await render({
      initialView: 'runs',
      scope: 'assistente',
      data: {
        ...towerFixture,
        runs: [],
        errors: { ...towerFixture.errors, board: 'Kanban unavailable' },
      },
    })
    expect(container.textContent).toContain('Run card links could not be listed')
  })

  it('shows absent prose slots, top three Now items, and transcript fallbacks without a primary action', async () => {
    await render()
    expect(container.textContent).toContain('No summary for this project')
    expect(container.textContent).toContain('No diary for this project yet')
    expect(container.textContent).toContain('from the transcript')
    expect(
      Array.from(container.querySelectorAll('[data-urgency="now"] button')).filter(
        (node) => node.textContent?.trim() === 'Open',
      ),
    ).toHaveLength(3)
    expect(container.querySelector('button[data-variant="default"]')).toBeNull()
    await click('Open')
    expect(container.textContent).toContain('Or answer in your own words')
    expect(container.textContent).toContain(towerFixture.items[0].title)
  })

  it('isolates GitHub failure and uses a dash for the Open PRs tile', async () => {
    await render({
      data: {
        ...towerFixture,
        prs: [],
        errors: { ...towerFixture.errors, prs: 'unreachable' },
      },
    })
    expect(container.textContent).toContain('Pull requests are not listed')
    expect(container.textContent).toContain('Assistente · R1')
    expect(container.textContent).toContain('Board')
    const tile = Array.from(container.querySelectorAll('.tower-tile')).find((node) =>
      node.textContent?.includes('Open PRs'),
    )
    expect(tile?.querySelector('strong')?.textContent).toBe('—')
  })

  it('keeps available Needs you items visible when another source fails', async () => {
    await render({
      data: {
        ...towerFixture,
        errors: { ...towerFixture.errors, needsYou: 'Approval is unavailable' },
      },
    })
    expect(container.textContent).toContain('Some Needs you sources are unavailable')
    expect(container.textContent).toContain(towerFixture.items[0].title)
    const waiting = Array.from(container.querySelectorAll('.tower-tile')).find((node) =>
      node.textContent?.trim().startsWith('Waiting'),
    )
    expect(waiting?.querySelector('strong')?.textContent).toBe('—')
  })

  it('clears the selected question when it leaves the project scope', async () => {
    await render()
    await click('Open')
    expect(container.textContent).toContain(towerFixture.items[0].title)
    await render({ scope: 'empty', data: { ...towerFixture, items: [] } })
    expect(container.textContent).not.toContain(towerFixture.items[0].title)
  })

  it('shows a healthy wake as waiting for wake and excludes it from the Waiting tile', async () => {
    const run = towerFixture.runs[0]
    const data = {
      ...towerFixture,
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
    const tile = Array.from(container.querySelectorAll('.tower-tile')).find((node) =>
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

  it('shows a failed progress read without claiming the card has no subtasks', async () => {
    await render({
      initialView: 'runs',
      data: {
        ...towerFixture,
        runs: [{ ...towerFixture.runs[0], subtasks: null, progress: [] }],
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
        ...towerFixture,
        errors: {
          ...towerFixture.errors,
          sessions: 'session-manager unavailable',
        },
        refresh,
      },
    })
    expect(container.textContent).toContain('Control tower')
    expect(container.textContent).toContain('Sessions could not be listed')
    await click('Retry')
    expect(refresh).toHaveBeenCalledOnce()
  })
})

describe('CEO overview', () => {
  it('orders direction, urgency, exceptions and new assumptions before the folded details', async () => {
    await render({ summary: writer() })
    const headings = [...container.querySelectorAll('h2')].map((node) => node.textContent)
    expect(headings.slice(0, 4)).toEqual(['Direction', 'Needs you', 'Exceptions', 'Decided for you'])
    expect(container.textContent).toContain('At risk')
    expect(container.textContent).toContain('Open cards near the target date')
    expect(container.textContent).toContain('An escalated question is open')
    expect(container.textContent).toContain('KAN-597 holds the release on build isolation.')
    expect(container.querySelector('[data-urgency="now"]')?.textContent).toContain('build machine')
    expect(container.querySelector('[data-urgency="no-rush"]')?.textContent).toContain('Verify timed out twice')
    expect(container.querySelector('[data-urgency="now"]')?.textContent).not.toContain('Verify timed out twice')
    expect(
      [...container.querySelectorAll('details.tower-folded')].map((node) => node.querySelector('summary')?.textContent),
    ).toEqual(['Releases', 'Runs in full', 'Pull requests', 'Board', 'Diary'])
    expect(container.querySelector('details[open]')).toBeNull()
  })

  it('shows past-target and on-track facts from the read model', async () => {
    await render({
      data: {
        ...towerFixture,
        direction: [
          { release: towerFixture.releases[1], state: 'at-risk', reasons: ['past-target'] },
          { release: towerFixture.releases[2], state: 'on-track', reasons: [] },
        ],
      },
    })
    expect(container.textContent).toContain('Target date has passed')
    expect(container.textContent).toContain('On track')
  })

  it('shows only assumptions returned since the last visit and the decision wait median', async () => {
    await render({ data: { ...towerFixture, assumptionsSinceVisit: [] } })
    expect(container.textContent).toContain('No new assumptions since your last visit')
    expect(container.textContent).not.toContain('Cache artifacts between builds?')
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
    expect(container.textContent).toContain('update failed')
    expect(container.textContent).toContain('Last valid summary')
    expect(container.textContent).toContain(towerSummaryFixture.brief.headline)
    expect(container.textContent).toContain('Refresh available')
    expect(container.querySelector('.tower-freshness .bg-warn')).not.toBeNull()
  })

  it.each([
    'writer profile missing: apply the tower kit',
    'set the summary model in the tower settings',
  ])('shows %s', async (error) => {
    await render({ summary: { ...writer(), result: null, error, fresh: false, unavailable: true } })
    expect(container.textContent).toContain(error)
  })

  it('shows stale PR checks and omits subtree usage metrics', async () => {
    await render({ initialView: 'overview' })
    expect(container.textContent).toContain('Checked 12m ago')
    await click('Runs · 4')
    expect(container.textContent).not.toContain('Usage')
    expect(container.textContent).not.toContain('17M')
  })
})

describe('last visit per browser', () => {
  it('passes the previous visit to the hook, stores this visit and keeps the read cutoff stable', async () => {
    const previous = new Date(Date.now() - 60 * 60_000).toISOString()
    localStorage.setItem('tower:last-visit:all', JSON.stringify(previous))
    const hook = vi.spyOn(overviewHook, 'useTowerOverview').mockReturnValue(towerFixture)
    try {
      await act(() => root.render(<Tower host={host} paneId="visit-test" tabId="test" panelSide="left" />))
      expect(hook).toHaveBeenLastCalledWith(host, 'all', previous)
      const stored = JSON.parse(localStorage.getItem('tower:last-visit:all') ?? 'null')
      expect(Date.parse(stored)).toBeGreaterThan(Date.parse(previous))
      await act(() => root.render(<Tower host={host} paneId="visit-test" tabId="test" panelSide="left" />))
      expect(hook).toHaveBeenLastCalledWith(host, 'all', previous)
      await act(() => root.render(null))
    } finally {
      hook.mockRestore()
    }
  })
})
