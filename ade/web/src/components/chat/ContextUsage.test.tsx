// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { ContextUsage } from './ContextUsage'

vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)

let root: Root | null = null
afterEach(() => {
  act(() => root?.unmount())
  root = null
  document.body.replaceChildren()
})

function mount(node: React.ReactNode) {
  const container = document.body.appendChild(document.createElement('div'))
  root = createRoot(container)
  act(() => root?.render(node))
  return container
}

// QA 2026-11-16: in a ~245px chat pane the meter's "21.4k / 852k" pushed
// over the back arrow and the chat's title vanished. Under a 24rem pane the
// counts give way (the chat root is the `@container`); the ring stays, and
// the trigger's label keeps every number.
describe('ContextUsage in a narrow pane', () => {
  it('drops the counts under 24rem and keeps the ring and the label', () => {
    const container = mount(
      <ContextUsage
        messages={[]}
        reported={{ total: 21_400, usable: 852_000, free: 830_600 }}
      />,
    )
    const trigger = container.querySelector('button')
    expect(trigger?.getAttribute('aria-label')).toContain('21,400 tokens')
    const counts = [...(trigger?.querySelectorAll('span') ?? [])].find((el) =>
      el.textContent?.includes('852'),
    )
    expect(counts?.className).toContain('@max-[24rem]:hidden')
    const ring = trigger?.querySelector('[role="progressbar"]')
    expect(ring?.className).not.toContain('hidden')
  })

  it('does the same for a bare count with no window', () => {
    const container = mount(<ContextUsage messages={[]} />)
    const trigger = container.querySelector('button')
    const count = [...(trigger?.querySelectorAll('span') ?? [])].find(
      (el) =>
        el.getAttribute('role') !== 'progressbar' &&
        /\d/.test(el.textContent ?? ''),
    )
    expect(count?.className).toContain('@max-[24rem]:hidden')
  })
})
