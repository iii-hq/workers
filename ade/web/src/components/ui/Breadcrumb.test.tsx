// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { Breadcrumb } from './Breadcrumb'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const cleanups: Array<() => void> = []
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup()
  vi.restoreAllMocks()
})

function render(node: React.ReactNode) {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  act(() => root.render(node))
  cleanups.push(() => {
    act(() => root.unmount())
    container.remove()
  })
  return container
}

describe('Breadcrumb', () => {
  // QA 2026-11-16: opening a function on the Functions page logged
  // "<li> cannot be a descendant of <li>": each separator sat inside the
  // item after it.
  it('puts separators beside the items, never inside one', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    const container = render(
      <Breadcrumb
        items={[
          { label: 'functions', onClick: () => {} },
          { label: 'state' },
          { label: 'list_groups' },
        ]}
      />,
    )
    expect(container.querySelectorAll('li li')).toHaveLength(0)
    const list = container.querySelector('ol')
    expect(
      [...(list?.children ?? [])].map((li) => li.getAttribute('data-slot')),
    ).toEqual([
      'breadcrumb-item',
      'breadcrumb-separator',
      'breadcrumb-item',
      'breadcrumb-separator',
      'breadcrumb-item',
    ])
    expect(container.querySelector('[aria-current="page"]')?.textContent).toBe(
      'list_groups',
    )
    expect(error).not.toHaveBeenCalled()
  })
})
