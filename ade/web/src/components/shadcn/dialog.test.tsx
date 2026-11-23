// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it } from 'vitest'
import { Dialog, DialogContent, DialogTitle } from './dialog'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const cleanups: Array<() => void> = []
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup()
})

function mount(className?: string) {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  act(() =>
    root.render(
      <Dialog open>
        <DialogContent className={className}>
          <DialogTitle>Context</DialogTitle>
          <div data-card="">
            <span style={{ whiteSpace: 'nowrap' }}>≥ $76.96 ≥ $166.96</span>
          </div>
        </DialogContent>
      </Dialog>,
    ),
  )
  cleanups.push(() => {
    act(() => root.unmount())
    container.remove()
  })
  const sheet = document.querySelector<HTMLElement>(
    '[data-slot="dialog-content"]',
  )
  if (!sheet) throw new Error('dialog sheet did not mount')
  return sheet
}

describe('DialogContent sheet', () => {
  // The sheet is a one-column grid. Its column must be `minmax(0, 1fr)`:
  // the implicit `auto` track grows to the content's min-content width, so
  // a worker's card with one nowrap value (the harness context breakdown's
  // cost tile, its models table) widened past the sheet's max-width and
  // rendered clipped on both sides. With a zero minimum the card is as
  // wide as the sheet and its own truncation rules apply.
  it('caps its single column at the sheet width so content cannot widen it', () => {
    const sheet = mount()
    expect(sheet.className).toContain('grid')
    expect(sheet.className).toContain('grid-cols-[minmax(0,1fr)]')
  })

  it('still lets a caller choose its own columns', () => {
    const sheet = mount('grid-cols-2')
    expect(sheet.className).toContain('grid-cols-2')
    expect(sheet.className).not.toContain('grid-cols-[minmax(0,1fr)]')
  })
})
