import type { ReactElement, ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { ContextMenuSurface } from '../ContextMenu'

// The props each menu item rendered with, to select it.
const items = vi.hoisted(() => [] as Array<{ onSelect: () => void }>)
// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => {
  const Pass = ({ children }: { children?: ReactNode }) => <>{children}</>
  return {
    DropdownMenu: Pass,
    DropdownMenuContent: Pass,
    DropdownMenuTrigger: Pass,
    DropdownMenuItem: (props: { onSelect: () => void; children?: ReactNode }) => {
      items.push(props)
      return <>{props.children}</>
    },
    DropdownMenuLabel: ({ className, children }: { className?: string; children?: ReactNode }) => (
      <div className={className}>{children}</div>
    ),
  }
})
// Where each portal goes; server rendering has no portals.
const portals = vi.hoisted(() => [] as Array<{ node: ReactElement; target: unknown }>)
vi.mock('react-dom', async (original) => ({
  ...(await original<typeof import('react-dom')>()),
  createPortal: (node: ReactElement, target: unknown) => {
    portals.push({ node, target })
    return node
  },
}))

afterEach(() => {
  portals.length = 0
  items.length = 0
  vi.unstubAllGlobals()
})

describe('the context menu anchor', () => {
  it('sits in the document body at the pointer, outside the page frame', () => {
    // The IDE frame is a size container, which makes it the containing block
    // of every fixed box inside it: an anchor there lands off by the pane's offset.
    const body = { tagName: 'BODY' }
    vi.stubGlobal('document', { body })
    const html = renderToStaticMarkup(
      <ContextMenuSurface state={{ anchor: { x: 748, y: 285 }, items: [] }} onClose={() => {}} />,
    )
    expect(portals).toHaveLength(1)
    expect(portals[0].target).toBe(body)
    expect(html).toContain('position:fixed;left:748px;top:285px')
  })
})

describe('the context menu', () => {
  it('runs the chosen action after it has closed', async () => {
    vi.stubGlobal('document', { body: {} })
    const log: string[] = []
    renderToStaticMarkup(
      <ContextMenuSurface
        state={{ anchor: { x: 0, y: 0 }, items: [{ id: 'go', label: 'Go', onSelect: () => log.push('action') }] }}
        onClose={() => log.push('close')}
      />,
    )
    items[0].onSelect()
    // An action that moves focus would lose it to the open menu.
    expect(log).toEqual(['close'])
    await Promise.resolve()
    expect(log).toEqual(['close', 'action'])
  })

  it('heads the menu with what it acts on, its detail only when there is one', () => {
    vi.stubGlobal('document', { body: {} })
    const head = (detail?: string) =>
      renderToStaticMarkup(
        <ContextMenuSurface
          state={{ anchor: { x: 0, y: 0 }, items: [{ type: 'label', id: 'head', label: 'a.ts', icon: <i />, detail }] }}
          onClose={() => {}}
        />,
      )
    expect(head('src')).toContain(
      '<div class="shui-context-head"><span class="menu-icon" aria-hidden="true"><i></i></span><span class="menu-label">a.ts</span><span class="menu-detail">src</span></div>',
    )
    expect(head()).toContain('<span class="menu-label">a.ts</span></div>')
    expect(head()).not.toContain('menu-detail')
  })
})
