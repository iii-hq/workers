// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { resetFoldedGroups } from '@/lib/context-groups'
import { ContextSectionsMenu } from './ContextSectionsMenu'

let root: Root | null = null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  window.localStorage.clear()
  resetFoldedGroups()
  root = createRoot(document.body.appendChild(document.createElement('div')))
  act(() =>
    root?.render(
      <TooltipProvider delayDuration={0}>
        <ContextSectionsMenu phone={false} />
      </TooltipProvider>,
    ),
  )
})

afterEach(() => {
  act(() => root?.unmount())
  root = null
  document.body.innerHTML = ''
  vi.unstubAllGlobals()
})

const button = (label: string) =>
  [...document.querySelectorAll('button')].find(
    (el) =>
      el.getAttribute('aria-label') === label ||
      el.textContent?.trim() === label,
  )
const open = () => {
  const trigger = button('Sections') as HTMLButtonElement
  act(() => {
    trigger.dispatchEvent(
      new PointerEvent('pointerdown', { bubbles: true, button: 0 }),
    )
    trigger.click()
  })
}
const names = () =>
  [...document.querySelectorAll('li[data-reorder-item]')].map((li) =>
    li.getAttribute('data-reorder-item'),
  )
const stored = (key: string): string[] =>
  JSON.parse(window.localStorage.getItem(key) ?? '[]')

describe('the Sections menu', () => {
  it('lists every section in panel order, ticked when shown', () => {
    open()
    expect(names()).toEqual([
      'project',
      'tickets',
      'prs',
      'assets',
      'subagents',
      'triggers',
      'links',
      'tracking',
    ])
    const links = document.querySelector(
      'li[data-reorder-item="links"] input[type="checkbox"]',
    ) as HTMLInputElement
    expect(links.checked).toBe(true)
    act(() => links.click())
    expect(stored('iii-context-hidden')).toEqual(['links'])
    expect(links.checked).toBe(false)
  })

  it('moves a section with its handle and resets the order', () => {
    open()
    const handle = button('Move Links') as HTMLButtonElement
    act(() => {
      handle.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Home', bubbles: true }),
      )
    })
    expect(names()[0]).toBe('links')
    expect(stored('iii-context-order')[0]).toBe('links')
    act(() => (button('Reset to default order') as HTMLButtonElement).click())
    expect(names()[0]).toBe('project')
    expect(stored('iii-context-order')).toEqual([])
  })
})
