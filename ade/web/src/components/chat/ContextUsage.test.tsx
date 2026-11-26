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

// chat-projects §3.3.3: the amber tick at effective / usable on the ring and
// the bar, "auto 400k" after the counts, and the line in the breakdown. At
// the model's own limit (a null effective) there is no tick and no label.
describe('ContextUsage auto-compact', () => {
  const reported = { total: 142_000, usable: 916_000, free: 774_000 }

  it('marks the ring and labels the counts', () => {
    const container = mount(
      <ContextUsage
        messages={[]}
        reported={reported}
        autocompact={{ effective: 400_000, rule: { scope: 'global' } }}
      />,
    )
    const trigger = container.querySelector('button')
    expect(trigger?.querySelector('[data-ring-tick]')).not.toBeNull()
    const label = trigger?.querySelector('[data-autocompact-label]')
    expect(label?.textContent).toBe('·auto 400k')
    // Wide surfaces only: a narrow pane keeps the tick.
    expect(label?.className).toContain('@max-[30rem]:hidden')
    expect(trigger?.getAttribute('aria-label')).toContain(', auto 400k')
  })

  it('writes the line and marks the bar in the breakdown', () => {
    const container = mount(
      <ContextUsage
        messages={[]}
        reported={reported}
        autocompact={{
          effective: 160_000,
          rule: { scope: 'project', project_name: 'acme-web' },
        }}
      />,
    )
    act(() => container.querySelector('button')?.click())
    expect(document.querySelector('[data-autocompact-line]')?.textContent).toBe(
      'Auto-compact at 160k · acme-web rule',
    )
    expect(
      document.querySelector('[data-slot="fact-meter-tick"]'),
    ).not.toBeNull()
  })

  it("has no tick and no label at the model's limit, and says so", () => {
    const container = mount(
      <ContextUsage
        messages={[]}
        reported={reported}
        autocompact={{ effective: null, rule: { scope: 'global' } }}
      />,
    )
    const trigger = container.querySelector('button')
    expect(trigger?.querySelector('[data-ring-tick]')).toBeNull()
    expect(trigger?.querySelector('[data-autocompact-label]')).toBeNull()
    act(() => trigger?.click())
    expect(document.querySelector('[data-autocompact-line]')?.textContent).toBe(
      "At the model's limit",
    )
    expect(document.querySelector('[data-slot="fact-meter-tick"]')).toBeNull()
  })

  it('shows nothing of it on a harness without auto-compact', () => {
    const container = mount(<ContextUsage messages={[]} reported={reported} />)
    const trigger = container.querySelector('button')
    expect(trigger?.querySelector('[data-ring-tick]')).toBeNull()
    act(() => trigger?.click())
    expect(document.querySelector('[data-autocompact-line]')).toBeNull()
  })
})
