// @vitest-environment jsdom
import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { requestPanelOpen, requestScreenOpen } from '@/lib/panel-context'
import { TowerScreen } from '.'
import { towerFixture } from './fixtures'

vi.mock('@/lib/panel-context', () => ({
  requestPanelOpen: vi.fn(),
  requestScreenOpen: vi.fn(),
}))
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
        <TowerScreen
          data={towerFixture}
          scope="all"
          onScopeChange={() => {}}
          {...props}
        />
      </TooltipProvider>,
    ),
  )
}
async function click(label: string) {
  const button = Array.from(container.querySelectorAll('button')).find(
    (node) => node.textContent?.trim() === label,
  )
  expect(button, label).toBeDefined()
  await act(() => button?.click())
}

describe('Control tower views', () => {
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
    expect(container.textContent).toContain(
      'Run card links could not be listed',
    )
  })

  it('shows absent prose slots, top three Now items, and transcript fallbacks without a primary action', async () => {
    await render()
    expect(container.textContent).toContain('No summary for this project')
    expect(container.textContent).toContain('No diary for this project yet')
    expect(container.textContent).toContain('from the transcript')
    expect(
      Array.from(container.querySelectorAll('button')).filter(
        (node) => node.textContent?.trim() === 'Open',
      ),
    ).toHaveLength(3)
    expect(container.querySelector('button[data-variant="default"]')).toBeNull()
    await click('Open')
    expect(container.textContent).toContain(
      'the item detail will be available here',
    )
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
    const tile = Array.from(container.querySelectorAll('.tower-tile')).find(
      (node) => node.textContent?.includes('Open PRs'),
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
    expect(container.textContent).toContain(
      'Some Needs you sources are unavailable',
    )
    expect(container.textContent).toContain(towerFixture.items[0].title)
    const waiting = Array.from(container.querySelectorAll('.tower-tile')).find(
      (node) => node.textContent?.startsWith('Waiting'),
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
    const tile = Array.from(container.querySelectorAll('.tower-tile')).find(
      (node) => node.textContent?.startsWith('Waiting'),
    )
    expect(tile?.querySelector('strong')?.textContent).toBe('0')
  })

  it('labels progress bars and opens the real session and card screens', async () => {
    await render({ initialView: 'runs' })
    expect(
      container
        .querySelector('[role="progressbar"]')
        ?.getAttribute('aria-label'),
    ).toContain('11 of 14 done, 2 in progress')
    expect(container.textContent).toContain('not started')
    await click('Open session')
    expect(requestScreenOpen).toHaveBeenCalledWith({
      screen: 'chat:dispatch-r1',
      relativeTo: 'tower',
    })
    await click('Open card')
    expect(requestPanelOpen).toHaveBeenCalledWith({
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
