// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  BottomSheet,
  BottomSheetClose,
  BottomSheetTrigger,
} from './BottomSheet'

vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)

let root: Root | null = null
afterEach(() => {
  act(() => root?.unmount())
  root = null
  document.body.replaceChildren()
  vi.restoreAllMocks()
})

function mount(node: React.ReactNode) {
  const container = document.body.appendChild(document.createElement('div'))
  root = createRoot(container)
  act(() => root?.render(node))
  return container
}

// The public console-ui API types these with `DialogTriggerProps` (Radix
// `asChild`); the harness context chip uses it on phones. Base UI spells it
// `render`, so passing `asChild` through nested a button in a button and
// leaked `asChild` onto the DOM (QA 2026-11-16, three errors per phone load).
describe('BottomSheet trigger and close (legacy asChild)', () => {
  it('makes the child the trigger: one button, no stray prop, no warnings', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    const container = mount(
      <BottomSheet>
        <BottomSheetTrigger asChild>
          <button type="button" className="chip" aria-label="context">
            12k
          </button>
        </BottomSheetTrigger>
      </BottomSheet>,
    )
    const buttons = container.querySelectorAll('button')
    expect(buttons).toHaveLength(1)
    expect(buttons[0].className).toBe('chip')
    expect(buttons[0].getAttribute('aria-haspopup')).toBe('dialog')
    expect(container.querySelector('[aschild]')).toBeNull()
    expect(container.querySelectorAll('button button')).toHaveLength(0)
    expect(error).not.toHaveBeenCalled()
  })

  it('does the same for the close control', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    const container = mount(
      <BottomSheet>
        <BottomSheetClose asChild>
          <button type="button" className="done">
            Done
          </button>
        </BottomSheetClose>
      </BottomSheet>,
    )
    expect(container.querySelectorAll('button')).toHaveLength(1)
    expect(container.querySelector('[aschild]')).toBeNull()
    expect(error).not.toHaveBeenCalled()
  })

  it('still renders its own button without asChild', () => {
    const container = mount(
      <BottomSheet>
        <BottomSheetTrigger>Open</BottomSheetTrigger>
      </BottomSheet>,
    )
    const buttons = container.querySelectorAll('button')
    expect(buttons).toHaveLength(1)
    expect(buttons[0].textContent).toBe('Open')
  })
})
