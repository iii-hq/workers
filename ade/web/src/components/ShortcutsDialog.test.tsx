// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { KEYBINDINGS } from '@/lib/keybindings/registry'
import { orderedShortcutGroups, ShortcutsDialog } from './ShortcutsDialog'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

describe('orderedShortcutGroups', () => {
  it('lists every registry group once, from anywhere to inside the palette', () => {
    const groups = orderedShortcutGroups('mac')
    expect(groups.map((group) => group.group)).toEqual([
      'ADE',
      'Chat',
      'Go to',
      'Workspace',
      'Search',
    ])
    const listed = groups.flatMap((group) => group.entries.map((e) => e.id))
    expect(new Set(listed).size).toBe(listed.length)
    expect(listed.sort()).toEqual(KEYBINDINGS.map((d) => d.id).sort())
  })

  it('describes each group with where its keys work', () => {
    for (const group of orderedShortcutGroups('other')) {
      expect(group.detail.length).toBeGreaterThan(0)
      expect(group.entries.length).toBeGreaterThan(0)
    }
  })
})

describe('ShortcutsDialog', () => {
  let root: Root
  let host: HTMLDivElement

  beforeEach(() => {
    host = document.createElement('div')
    document.body.appendChild(host)
    root = createRoot(host)
  })

  afterEach(() => {
    act(() => root.unmount())
    host.remove()
  })

  it('renders every shortcut under its group with key caps', async () => {
    const onOpenChange = vi.fn()
    await act(async () => {
      root.render(<ShortcutsDialog open onOpenChange={onOpenChange} />)
    })
    const dialog = document.body.querySelector('[role="dialog"]')
    expect(dialog).not.toBeNull()
    expect(dialog?.textContent).toContain('Keyboard shortcuts')
    for (const definition of KEYBINDINGS) {
      expect(dialog?.textContent).toContain(definition.title)
    }
    expect(dialog?.querySelectorAll('kbd').length).toBeGreaterThan(
      KEYBINDINGS.length,
    )
    for (const heading of ['ADE', 'Chat', 'Go to', 'Workspace', 'Search']) {
      expect(
        [...(dialog?.querySelectorAll('h3') ?? [])].some(
          (h) => h.textContent === heading,
        ),
      ).toBe(true)
    }
  })
})
