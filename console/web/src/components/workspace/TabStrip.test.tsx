import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { WorkspaceTab } from '@/lib/workspace-tabs'
import { TabStrip, tabIndexForKey } from './TabStrip'

const tabs: WorkspaceTab[] = [
  { id: 'a', screens: ['chat', 'traces'] },
  { id: 'b', name: 'IDE', screens: ['ext:shell'] },
  { id: 'c', screens: ['workers'] },
]

function markup(activeTabId = 'b') {
  return renderToStaticMarkup(
    <TabStrip
      tabs={tabs}
      activeTabId={activeTabId}
      extPageTitles={new Map([['shell', 'Shell']])}
      onActivate={vi.fn()}
      onClose={vi.fn()}
      onCreate={vi.fn()}
      onRename={vi.fn()}
      onReorder={vi.fn()}
    />,
  )
}

describe('tabIndexForKey', () => {
  it('moves with the arrow keys and wraps at both ends', () => {
    expect(tabIndexForKey('ArrowRight', 0, 3)).toBe(1)
    expect(tabIndexForKey('ArrowRight', 2, 3)).toBe(0)
    expect(tabIndexForKey('ArrowLeft', 0, 3)).toBe(2)
  })

  it('jumps to the ends with Home and End', () => {
    expect(tabIndexForKey('Home', 2, 3)).toBe(0)
    expect(tabIndexForKey('End', 0, 3)).toBe(2)
  })

  it('ignores every other key and an empty strip', () => {
    expect(tabIndexForKey('Enter', 1, 3)).toBeNull()
    expect(tabIndexForKey('ArrowRight', 0, 0)).toBeNull()
  })
})

describe('TabStrip markup', () => {
  it('keeps one tab in the focus order: the active one', () => {
    const html = markup()
    expect(html.match(/role="tab"/g)).toHaveLength(3)
    expect(html.match(/role="tab"[^>]*tabindex="0"/g)).toHaveLength(1)
    expect(html).toContain('data-tab-id="b"')
    expect(html).toMatch(/data-tab-id="b"[^>]*aria-selected="true"/)
  })

  it('names the controls in natural case', () => {
    const html = markup()
    expect(html).toContain('aria-label="Workspace tabs"')
    expect(html).toContain('aria-label="New workspace"')
    expect(html).toContain('aria-label="Close IDE"')
  })

  it('renders no overflow menu before the strip has been measured', () => {
    expect(markup()).not.toContain('All workspaces')
  })
})
