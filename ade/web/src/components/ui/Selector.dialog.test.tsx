// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it } from 'vitest'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from './Dialog'
import { Selector } from './Selector'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

// A catalog long enough to scroll, like a model picker's.
const options = Array.from({ length: 40 }, (_, index) => ({
  value: `model-${index}`,
  label: `Model ${index}`,
}))

const mounted: Array<() => void> = []

afterEach(() => {
  for (const unmount of mounted.splice(0)) unmount()
})

function mountInDialog() {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  act(() =>
    root.render(
      <Dialog open>
        <DialogContent>
          <DialogTitle>Investigate with…</DialogTitle>
          <DialogDescription>For this investigation only.</DialogDescription>
          <Selector
            aria-label="Model"
            value={undefined}
            options={options}
            onChange={() => {}}
            placeholder="pick a model"
            searchPlaceholder="model or provider"
          />
        </DialogContent>
      </Dialog>,
    ),
  )
  mounted.push(() => {
    act(() => root.unmount())
    container.remove()
  })
  const trigger = document.querySelector<HTMLButtonElement>(
    'button[aria-label="Model"]',
  )
  act(() => {
    trigger?.dispatchEvent(
      new PointerEvent('pointerdown', { bubbles: true, button: 0 }),
    )
    trigger?.click()
  })
}

describe('Selector inside a modal Dialog', () => {
  it('takes focus into its search, so the keyboard reaches the list', () => {
    mountInDialog()
    const search = document.querySelector(
      'input[aria-label="model or provider"]',
    )
    expect(search).not.toBeNull()
    // With two copies of Radix's focus scope the dialog's trap handed focus
    // straight back to the trigger: typing filtered nothing.
    expect(document.activeElement).toBe(search)
  })

  it('lets its own list scroll although the dialog locks scrolling', () => {
    mountInDialog()
    const list = document.querySelector<HTMLElement>('[role="listbox"]')
    expect(list).not.toBeNull()
    if (!list) return
    // jsdom has no layout: give the list the overflow it has in a browser.
    list.style.overflowY = 'auto'
    Object.defineProperty(list, 'scrollHeight', {
      configurable: true,
      value: 2000,
    })
    Object.defineProperty(list, 'clientHeight', {
      configurable: true,
      value: 400,
    })
    const wheel = new WheelEvent('wheel', {
      deltaY: 120,
      bubbles: true,
      cancelable: true,
    })
    list.dispatchEvent(wheel)
    // The list is portalled outside the dialog; without a scroll lock of its
    // own the dialog's lock cancelled every wheel over it.
    expect(wheel.defaultPrevented).toBe(false)
  })
})
