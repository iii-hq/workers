import type { ReactElement, ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { ContextMenuSurface } from '../ContextMenu'

// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => {
  const Pass = ({ children }: { children?: ReactNode }) => <>{children}</>
  return { DropdownMenu: Pass, DropdownMenuContent: Pass, DropdownMenuTrigger: Pass }
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
