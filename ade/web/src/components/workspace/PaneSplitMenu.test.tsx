// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { PageHeader } from '@/components/ui/PageChrome'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { type PaneActions, PaneActionsProvider } from '@/lib/pane-actions'

;(
  globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  act(() => root.unmount())
  container.remove()
  document.body.innerHTML = ''
})

function actions(overrides: Partial<PaneActions> = {}): PaneActions {
  return {
    split: vi.fn(),
    openScreens: ['ext:ide'],
    busy: false,
    ...overrides,
  }
}

async function renderHeader(paneActions?: PaneActions) {
  await act(async () => {
    root.render(
      <TooltipProvider>
        <PaneActionsProvider actions={paneActions}>
          <PageHeader title="IDE" onClose={() => {}} />
        </PaneActionsProvider>
      </TooltipProvider>,
    )
  })
}

const trigger = () =>
  container.querySelector<HTMLButtonElement>('[data-pane-split-trigger]')

async function openMenu() {
  await act(async () => {
    trigger()?.click()
  })
}

const items = () =>
  [...document.querySelectorAll('[role="menuitem"]')].map((item) =>
    item.textContent?.replace(/\s+/g, ' ').trim(),
  )

const item = (label: string) =>
  [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(
    (candidate) => candidate.textContent?.startsWith(label),
  )

describe('the pane split control', () => {
  it('is absent outside a workspace pane', async () => {
    await renderHeader()
    expect(trigger()).toBeNull()
  })

  it('names itself in every pane header', async () => {
    await renderHeader(actions())
    expect(trigger()?.getAttribute('aria-label')).toBe('Split pane')
  })

  it('offers what the tab does not show yet, then the empty splits', async () => {
    await renderHeader(actions())
    await openMenu()
    const rows = items()
    expect(rows[0]).toBe('chat')
    expect(rows).not.toContain('IDE')
    expect(rows.some((row) => row?.startsWith('Split right'))).toBe(true)
    expect(rows.some((row) => row?.startsWith('Split left'))).toBe(true)
    // Alone in its tab: there is nothing to close.
    expect(rows).not.toContain('Close other panes')
  })

  it('opens the picked screen to the right of this pane', async () => {
    const paneActions = actions()
    await renderHeader(paneActions)
    await openMenu()
    await act(async () => item('chat')?.click())
    expect(paneActions.split).toHaveBeenCalledWith('right', 'chat')
  })

  it('splits empty, and closes the others when there are some', async () => {
    const closeOthers = vi.fn()
    const paneActions = actions({ closeOthers })
    await renderHeader(paneActions)
    await openMenu()
    await act(async () => item('Split left')?.click())
    expect(paneActions.split).toHaveBeenCalledWith('left')
    await openMenu()
    await act(async () => item('Close other panes')?.click())
    expect(closeOthers).toHaveBeenCalled()
  })

  it('holds still while a pane is moving', async () => {
    await renderHeader(actions({ busy: true }))
    await openMenu()
    expect(item('Split right')?.getAttribute('aria-disabled')).toBe('true')
  })
})
