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

  // Chats are found by what was said in them: a live hit replaces the
  // title-only row with its id and shows the message that matched under the
  // title, with the matched words marked.
  it('replaces a chat row with its live hit and shows the snippet', async () => {
    const asked: string[] = []
    const liveSearch = async ({ text }: { text: string }) => {
      asked.push(text)
      return [
        {
          id: 'chat:s_1',
          kind: 'chat',
          title: 'Setup notes',
          snippet: {
            text: 'we streamlined the ADE setup',
            ranges: [[3, 14]] as const,
          },
          run: () => {},
        } satisfies PaletteEntry,
      ]
    }
    const chats: PaletteEntry[] = [
      ...entries,
      { id: 'chat:s_1', kind: 'chat', title: 'Setup notes', run: () => {} },
    ]
    await act(async () => {
      root.render(
        <CommandPalette
          open
          onClose={() => {}}
          localEntries={chats}
          onOpenWorker={() => {}}
          onOpenFunction={() => {}}
          readInventory={readInventory}
          liveSearch={liveSearch}
          initialQuery="@stremlined"
        />,
      )
    })
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 200))
    })
    expect(asked).toEqual(['stremlined'])
    const rows = [...document.querySelectorAll('[data-palette-index]')]
    expect(rows).toHaveLength(1)
    expect(rows[0].textContent).toContain('Setup notes')
    expect(rows[0].textContent).toContain('we streamlined the ADE setup')
    expect(rows[0].querySelector('mark')?.textContent).toBe('streamlined')
  })

  it('does not ask the live search for a one-letter query', async () => {
    const asked: string[] = []
    const liveSearch = async ({ text }: { text: string }) => {
      asked.push(text)
      return []
    }
    await act(async () => {
      root.render(
        <CommandPalette
          open
          onClose={() => {}}
          localEntries={entries}
          onOpenWorker={() => {}}
          onOpenFunction={() => {}}
          readInventory={readInventory}
          liveSearch={liveSearch}
          initialQuery="s"
        />,
      )
    })
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 200))
    })
    expect(asked).toEqual([])
  })

  // CodeRabbit on #1892: rows answered for one query must not stand once the
  // query changes, while the next answer is still in flight.
  it('drops live rows that answered an earlier query', async () => {
    const liveSearch = ({ text }: { text: string }) =>
      text === 'alpha'
        ? Promise.resolve([
            {
              id: 'chat:s_alpha',
              kind: 'chat',
              title: 'Alpha chat',
              run: () => {},
            } satisfies PaletteEntry,
          ])
        : new Promise<PaletteEntry[]>(() => {})
    await act(async () => {
      root.render(
        <CommandPalette
          open
          onClose={() => {}}
          localEntries={entries}
          onOpenWorker={() => {}}
          onOpenFunction={() => {}}
          readInventory={readInventory}
          liveSearch={liveSearch}
          initialQuery="@alpha"
        />,
      )
    })
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 200))
    })
    expect(document.body.textContent).toContain('Alpha chat')

    const input = document.querySelector('input') as HTMLInputElement
    const setValue = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value',
    )?.set
    await act(async () => {
      setValue?.call(input, '@beta')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 200))
    })
    expect(document.body.textContent).not.toContain('Alpha chat')
  })

  it('shows source rows without waiting for the chat search', async () => {
    const sources = [
      {
        key: 'files.files',
        scope: 'files',
        source: {
          id: 'files',
          title: 'Files',
          kind: 'file' as const,
          prefix: '#',
          search: async () => [{ id: 'r', title: 'report.md', run: () => {} }],
        },
      },
    ]
    await act(async () => {
      root.render(
        <CommandPalette
          open
          onClose={() => {}}
          localEntries={entries}
          onOpenWorker={() => {}}
          onOpenFunction={() => {}}
          readInventory={readInventory}
          sources={sources}
          liveSearch={() => new Promise<PaletteEntry[]>(() => {})}
          initialQuery="#report"
        />,
      )
    })
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 200))
    })
    expect(document.body.textContent).toContain('report.md')
  })
})
