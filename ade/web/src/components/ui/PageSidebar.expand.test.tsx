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

describe('PageSidebar collapseRequest', () => {
  it('ignores its first value and folds the column on each new one', () => {
    const props: PageSidebarProps = { collapsible: true, collapseRequest: 0 }
    const { render, aside } = mount(props)
    expect(aside().hasAttribute('data-collapsed')).toBe(false)
    render({ ...props, collapseRequest: 1 })
    expect(aside().hasAttribute('data-collapsed')).toBe(true)
    render({ ...props, collapseRequest: 1, expandRequest: 1 })
    expect(aside().hasAttribute('data-collapsed')).toBe(false)
  })

  it('closes the drawer when the pane is narrow, open and closed again', () => {
    const props: PageSidebarProps = {
      narrow: true,
      narrowMode: 'drawer',
      expandRequest: 0,
      collapseRequest: 0,
    }
    const { render, aside } = mount(props)
    render({ ...props, expandRequest: 1 })
    expect(aside().getAttribute('data-drawer')).toBe('open')
    render({ ...props, expandRequest: 1, collapseRequest: 1 })
    expect(aside().getAttribute('data-drawer')).toBe('closed')
    expect(aside().hasAttribute('data-collapsed')).toBe(true)
    render({ ...props, expandRequest: 2, collapseRequest: 1 })
    expect(aside().getAttribute('data-drawer')).toBe('open')
    render({ ...props, expandRequest: 2, collapseRequest: 2 })
    expect(aside().getAttribute('data-drawer')).toBe('closed')
  })

  it('opens the drawer below a header it must not cover', () => {
    const props: PageSidebarProps = {
      narrow: true,
      narrowMode: 'drawer',
      expandRequest: 0,
      drawerOffsetTop: 44,
    }
    const { render, aside } = mount(props)
    render({ ...props, expandRequest: 1 })
    const scrim = aside().querySelector<HTMLElement>(
      'button[aria-label="close side"]',
    )
    const panel = aside().querySelector<HTMLElement>('[tabindex="-1"][id]')
    const toggle = aside().querySelector<HTMLElement>(
      'button[aria-label="collapse side"]',
    )
    // The offset rides a CSS variable the scrim, the panel and the toggle
    // (8px lower) start from.
    expect(aside().style.getPropertyValue('--page-sidebar-drawer-top')).toBe(
      '44px',
    )
    expect(scrim?.className).toContain(
      'top-[var(--page-sidebar-drawer-top,0px)]',
    )
    expect(panel?.className).toContain(
      'top-[var(--page-sidebar-drawer-top,0px)]',
    )
    expect(toggle?.className).toContain(
      'top-[calc(var(--page-sidebar-drawer-top,0px)+0.5rem)]',
    )
  })

  it('leaves the caret where it was when the shortcut came from outside', () => {
    const outside = document.createElement('input')
    document.body.appendChild(outside)
    try {
      const props: PageSidebarProps = {
        narrow: true,
        narrowMode: 'drawer',
        expandRequest: 0,
        collapseRequest: 0,
      }
      const { render } = mount(props)
      render({ ...props, expandRequest: 1 })
      outside.focus()
      render({ ...props, expandRequest: 1, collapseRequest: 1 })
      expect(document.activeElement).toBe(outside)
    } finally {
      outside.remove()
    }
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
