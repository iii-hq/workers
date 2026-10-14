// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it } from 'vitest'
import { PageSidebar, type PageSidebarProps } from './PageSidebar'
import { TooltipProvider } from './Tooltip'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const roots: Array<{ root: Root; container: HTMLElement }> = []

function mount(props: PageSidebarProps) {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  roots.push({ root, container })
  const render = (next: PageSidebarProps) =>
    act(() =>
      root.render(
        <TooltipProvider delayDuration={0}>
          <PageSidebar label="side" {...next}>
            <button type="button">inside</button>
          </PageSidebar>
        </TooltipProvider>,
      ),
    )
  render(props)
  const aside = () => container.querySelector('aside') as HTMLElement
  return { render, aside }
}

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount())
    container.remove()
  }
})

describe('PageSidebar expandRequest', () => {
  it('ignores its first value and expands on each new one', () => {
    const { render, aside } = mount({
      collapsible: true,
      defaultCollapsed: true,
      expandRequest: 0,
    })
    expect(aside().hasAttribute('data-collapsed')).toBe(true)
    render({ collapsible: true, defaultCollapsed: true, expandRequest: 1 })
    expect(aside().hasAttribute('data-collapsed')).toBe(false)
  })

  it('opens the drawer when the pane is narrow', () => {
    const props: PageSidebarProps = {
      narrow: true,
      narrowMode: 'drawer',
      expandRequest: 0,
    }
    const { render, aside } = mount(props)
    expect(aside().getAttribute('data-drawer')).toBe('closed')
    render({ ...props, expandRequest: 1 })
    expect(aside().getAttribute('data-drawer')).toBe('open')
  })
})

describe('PageSidebar closeDrawerOnSelect', () => {
  const open = (closeDrawerOnSelect?: boolean) => {
    const props: PageSidebarProps = {
      narrow: true,
      narrowMode: 'drawer',
      expandRequest: 0,
      closeDrawerOnSelect,
    }
    const view = mount(props)
    view.render({ ...props, expandRequest: 1 })
    const inside = [...document.querySelectorAll('button')].find(
      (button) => button.textContent === 'inside',
    )
    act(() => inside?.click())
    return view.aside().getAttribute('data-drawer')
  }

  it('closes the drawer on a pick by default', () => {
    expect(open()).toBe('closed')
  })

  it('keeps the drawer open for a panel people work in', () => {
    expect(open(false)).toBe('open')
  })
})
