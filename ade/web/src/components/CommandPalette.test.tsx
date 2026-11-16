// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { PaletteEntry } from '@/lib/palette/sources'
import { CommandPalette } from './CommandPalette'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const readInventory = () =>
  Promise.resolve({ workers: [], functions: [], error: null })

const entries: PaletteEntry[] = [
  {
    id: 'command:chat.focus',
    kind: 'command',
    title: 'Focus message box',
    detail: 'Start typing a message',
    meta: 'Chat',
    run: () => {},
  },
  {
    id: 'page:browser',
    kind: 'page',
    title: 'Browser',
    detail: 'Browser sessions and automation.',
    meta: 'browser',
    metaMono: true,
    run: () => {},
  },
]

describe('CommandPalette row tags', () => {
  let host: HTMLDivElement
  let root: Root
  beforeEach(() => {
    host = document.createElement('div')
    document.body.append(host)
    root = createRoot(host)
  })
  afterEach(() => {
    act(() => root.unmount())
    host.remove()
  })

  // QA 2026-11-16 F11: the right-hand tag read "C…", "B…", "F." The tag's
  // own `max-w-[40%]` was a percentage of its shrink-wrapped slot — circular,
  // so it resolved to less than the word. The slot carries the 40% cap (of
  // the row) and the tag only truncates inside it.
  it('caps the tag slot at 40% of the row, not the tag at 40% of itself', async () => {
    await act(async () => {
      root.render(
        <CommandPalette
          open
          onClose={() => {}}
          localEntries={entries}
          onOpenWorker={() => {}}
          onOpenFunction={() => {}}
          readInventory={readInventory}
          initialQuery=">"
        />,
      )
    })
    const tags = [...document.querySelectorAll('[data-slot="item-actions"]')]
    expect(tags.length).toBeGreaterThan(0)
    for (const slot of tags) {
      expect(slot.className).toContain('max-w-[40%]')
      expect(slot.className).toContain('min-w-0')
      for (const tag of slot.querySelectorAll('span')) {
        expect(tag.className).not.toContain('max-w-[40%]')
      }
    }
    expect(document.body.textContent).toContain('Chat')
  })
})
