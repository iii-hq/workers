// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it } from 'vitest'
import { type ConfirmOptions, useConfirm } from './ConfirmDialog'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mounted: Array<() => void> = []

function mount() {
  let api!: ReturnType<typeof useConfirm>
  function Probe() {
    api = useConfirm()
    return <>{api.dialog}</>
  }
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  act(() => root.render(<Probe />))
  const unmount = () => {
    act(() => root.unmount())
    container.remove()
  }
  mounted.push(unmount)
  return {
    confirm(options: ConfirmOptions) {
      let promise!: Promise<boolean>
      act(() => {
        promise = api.confirm(options)
      })
      return promise
    },
    unmount,
  }
}

const dialog = () => document.querySelector('[role="alertdialog"]')
const button = (label: string) =>
  [...document.querySelectorAll('button')].find(
    (el) => el.textContent === label,
  )

afterEach(() => {
  for (const unmount of mounted.splice(0)) unmount()
})

describe('useConfirm', () => {
  it('renders nothing until asked, then resolves true on confirm', async () => {
    const { confirm } = mount()
    expect(dialog()).toBeNull()

    const answer = confirm({ title: 'Delete it?', confirmLabel: 'Delete' })
    expect(dialog()?.textContent).toContain('Delete it?')

    act(() => button('Delete')?.click())
    await expect(answer).resolves.toBe(true)
    expect(dialog()).toBeNull()
  })

  it('resolves false on cancel', async () => {
    const { confirm } = mount()
    const answer = confirm({ title: 'Discard?', cancelLabel: 'Keep' })
    act(() => button('Keep')?.click())
    await expect(answer).resolves.toBe(false)
  })

  it('resolves false on Escape', async () => {
    const { confirm } = mount()
    const answer = confirm({ title: 'Discard?' })
    act(() => {
      document.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }),
      )
    })
    await expect(answer).resolves.toBe(false)
  })

  it('cancels the open question when asked again, and on unmount', async () => {
    const { confirm, unmount } = mount()
    const first = confirm({ title: 'First?' })
    const second = confirm({ title: 'Second?' })
    await expect(first).resolves.toBe(false)
    expect(dialog()?.textContent).toContain('Second?')

    unmount()
    await expect(second).resolves.toBe(false)
  })
})
