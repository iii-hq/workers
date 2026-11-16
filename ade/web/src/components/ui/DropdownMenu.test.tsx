// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from './DropdownMenu'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const cleanups: Array<() => void> = []
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup()
  vi.restoreAllMocks()
})

function render(trigger: React.ReactElement) {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  act(() =>
    root.render(
      <DropdownMenu open>
        <DropdownMenuTrigger asChild>{trigger}</DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuItem>Open</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>,
    ),
  )
  cleanups.push(() => {
    act(() => root.unmount())
    container.remove()
  })
}

const nativeButtonWarnings = (spy: ReturnType<typeof vi.spyOn>) =>
  spy.mock.calls.filter((args: unknown[]) =>
    args.some((arg) => String(arg).includes('native <button>')),
  )

describe('DropdownMenuTrigger asChild', () => {
  // QA 2026-11-16: the IDE's right-click menu anchors to an invisible
  // <span>; Base UI logged "expected a native <button>" on every open.
  it('takes a non-button element without a console error', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    render(<span aria-hidden data-testid="anchor" />)
    expect(nativeButtonWarnings(error)).toEqual([])
    expect(document.body.textContent).toContain('Open')
  })

  it('still renders a button child as the native button trigger', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    render(<button type="button">More</button>)
    expect(nativeButtonWarnings(error)).toEqual([])
    const trigger = document.querySelector('button[aria-haspopup]')
    expect(trigger?.textContent).toBe('More')
  })
})
