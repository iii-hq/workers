// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it } from 'vitest'
import { ConfirmDialog, type ConfirmOptions, useConfirm } from './ConfirmDialog'

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

describe('ConfirmDialog with an action that takes time', () => {
  function mountDialog(onConfirm: () => Promise<void>) {
    const container = document.createElement('div')
    document.body.appendChild(container)
    const root = createRoot(container)
    let open = true
    const render = () =>
      root.render(
        <ConfirmDialog
          open={open}
          onOpenChange={(next) => {
            open = next
            render()
          }}
          title="Delete conversation?"
          description="It is permanently deleted. This cannot be undone."
          confirmLabel="Delete"
          busyLabel="Deleting…"
          tone="danger"
          onConfirm={onConfirm}
        />,
      )
    act(render)
    mounted.push(() => {
      act(() => root.unmount())
      container.remove()
    })
  }

  it('stays open and busy, then closes once the action is done', async () => {
    let finish!: () => void
    mountDialog(() => new Promise<void>((done) => (finish = done)))
    act(() => button('Delete')?.click())
    expect(button('Deleting…')?.disabled).toBe(true)
    expect(button('Cancel')?.disabled).toBe(true)
    expect(dialog()?.getAttribute('aria-busy')).toBe('true')
    await act(async () => finish())
    expect(dialog()).toBeNull()
  })

  it('says why it failed and offers to try again', async () => {
    let attempts = 0
    mountDialog(async () => {
      attempts += 1
      if (attempts === 1) throw new Error('the session is gone')
    })
    await act(async () => button('Delete')?.click())
    const alert = dialog()?.querySelector('[role="alert"]')
    expect(alert?.textContent).toBe('the session is gone')
    // Outside the scrolling text, so it shows however far that is scrolled.
    expect(
      dialog()
        ?.querySelector('h2')
        ?.parentElement?.contains(alert ?? null),
    ).toBe(false)
    expect(button('Close')).toBeDefined()
    await act(async () => button('Retry delete')?.click())
    expect(attempts).toBe(2)
    expect(dialog()).toBeNull()
  })

  it('scrolls a long text and keeps the actions on screen', () => {
    mountDialog(async () => {})
    const body = dialog()
    expect(body?.className).toContain('max-h-[85vh]')
    expect(body?.className).toContain('flex-col')
    const text = body?.querySelector('h2')?.parentElement
    expect(text?.className).toContain('overflow-y-auto')
    expect(text?.className).toContain('min-h-0')
    const actions = button('Cancel')?.parentElement
    expect(actions?.className).toContain('shrink-0')
    expect(text?.contains(actions ?? null)).toBe(false)
  })

  it('has a title, a description and the actions, nothing else', () => {
    mountDialog(async () => {})
    const body = dialog()
    expect(body?.querySelector('ul')).toBeNull()
    expect(
      [...(body?.querySelectorAll('button') ?? [])]
        .map((el) => el.textContent)
        // The corner ✕ is the dialog's own close control.
        .filter((label) => label !== '' && label !== 'Close'),
    ).toEqual(['Cancel', 'Delete'])
  })
})
