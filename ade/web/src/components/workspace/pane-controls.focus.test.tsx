// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { PanelDragHandle } from './pane-controls'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

let host: HTMLDivElement
let root: Root
let other: HTMLButtonElement
beforeEach(() => {
  host = document.createElement('div')
  other = document.createElement('button')
  document.body.append(host, other)
  root = createRoot(host)
})
afterEach(() => {
  act(() => root.unmount())
  host.remove()
  other.remove()
})

const handle = () =>
  host.querySelector<HTMLButtonElement>('button') as HTMLButtonElement
const frames = () =>
  act(() => new Promise<void>((resolve) => setTimeout(resolve, 60)))
const press = (key: string, target: Element = handle()) =>
  act(() => {
    target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }))
  })

function render(onMove = vi.fn(), index = 0) {
  act(() =>
    root.render(
      <PanelDragHandle
        index={index}
        count={3}
        onDragStart={vi.fn()}
        onDragEnd={vi.fn()}
        onMove={onMove}
      />,
    ),
  )
  return onMove
}

// QA 2026-11-16 F2: after one arrow move the focus left the handle (the
// moved chat pane's composer took it), so Home did nothing to the panel.
describe('PanelDragHandle focus', () => {
  it('keeps the focus through a move, so the next key moves the pane again', async () => {
    const onMove = render()
    act(() => handle().focus())
    press('ArrowRight')
    expect(onMove).toHaveBeenLastCalledWith(1)
    render(onMove, 1)
    // The moved pane's page takes the caret as it settles.
    act(() => other.focus())
    await frames()
    expect(document.activeElement).toBe(handle())
    press('Home')
    expect(onMove).toHaveBeenLastCalledWith(0)
  })

  it('lets go when the person moves on with the pointer', async () => {
    render()
    act(() => handle().focus())
    press('ArrowRight')
    act(() => {
      other.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
      other.focus()
    })
    await frames()
    expect(document.activeElement).toBe(other)
  })

  it('does not hold the focus when no move happened', async () => {
    render()
    act(() => handle().focus())
    act(() => other.focus())
    await frames()
    expect(document.activeElement).toBe(other)
  })
})
