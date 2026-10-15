// @vitest-environment jsdom

import { act, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { Dialog, DialogContent, DialogTitle } from './Dialog'
import { SearchField } from './SearchField'

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const cleanups: Array<() => void> = []
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup()
})

function mount(onOpenChange: (open: boolean) => void) {
  function Probe() {
    const [query, setQuery] = useState('')
    return (
      <Dialog open onOpenChange={onOpenChange}>
        <DialogContent>
          <DialogTitle>Sub-agents</DialogTitle>
          <SearchField aria-label="Filter" value={query} onChange={setQuery} />
        </DialogContent>
      </Dialog>
    )
  }
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  act(() => root.render(<Probe />))
  cleanups.push(() => {
    act(() => root.unmount())
    container.remove()
  })
  return document.querySelector<HTMLInputElement>('input[aria-label="Filter"]')!
}

function type(input: HTMLInputElement, text: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!
  act(() => {
    setter.call(input, text)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

function escape(input: HTMLInputElement) {
  act(() => {
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
  })
}

describe('SearchField inside a dialog', () => {
  it('clears the query on Escape and keeps the dialog open', () => {
    const onOpenChange = vi.fn()
    const input = mount(onOpenChange)
    input.focus()
    type(input, 'kan129')
    expect(input.value).toBe('kan129')
    escape(input)
    expect(input.value).toBe('')
    expect(onOpenChange).not.toHaveBeenCalled()
  })

  it('lets an Escape on an empty field close the dialog', () => {
    const onOpenChange = vi.fn()
    const input = mount(onOpenChange)
    input.focus()
    escape(input)
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })
})
