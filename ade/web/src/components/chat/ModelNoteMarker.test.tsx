// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { iiiMentionRuntime, setMentionRuntime } from '@/lib/mentions/runtime'
import type { SystemMessage } from '@/types/chat'
import { SystemNotice } from './SystemNotice'

vi.mock('@/lib/iii-client', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  getIiiClient: () => new Promise(() => {}),
}))

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  setMentionRuntime({
    listProviders: async () => [
      {
        v: 1,
        name: 'session',
        label: 'Sessions',
        icon: 'session',
        search: 'session::mention::search',
        getFunctionId: 'session::mention::get',
      },
    ],
    search: async () => [],
    get: async (_provider, id) => ({ id, label: 'hello' }),
  })
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  setMentionRuntime(iiiMentionRuntime)
  vi.unstubAllGlobals()
})

async function render(message: SystemMessage) {
  await act(async () => root.render(<SystemNotice message={message} />))
  for (let i = 0; i < 4; i++) {
    await act(async () => {
      await new Promise<void>((resolve) => setTimeout(resolve, 0))
    })
  }
}

function expand() {
  const trigger = container.querySelector<HTMLButtonElement>('button')
  return act(async () => trigger?.click())
}

describe('model note row', () => {
  it('is one quiet line whose disclosure shows the note as written', async () => {
    const text = '<memory bank="m">\n- likes tea\n- works on iii\n</memory>'
    await render({
      id: 'n1',
      role: 'system',
      kind: 'model-note',
      content: 'Note to the model — memory',
      note: { label: 'memory', text },
      createdAt: 1,
    })
    const row = container.querySelector('[data-message-role="model-note"]')
    expect(row?.querySelector('[data-message-summary]')?.textContent).toBe(
      'Note to the model · memory',
    )
    expect(
      container.querySelector('[data-message-role="system-notice"]'),
    ).toBeNull()
    await expand()
    // Line breaks survive: the note reads as the model read it.
    expect(container.querySelector('[data-model-note-text]')?.textContent).toBe(
      text,
    )
  })

  it('reads a mentions note as pills, summaries and details calls', async () => {
    await render({
      id: 'n2',
      role: 'system',
      kind: 'model-note',
      content: 'Mentions resolved for the model',
      note: {
        label: 'mentions',
        text: '<mentions>…</mentions>',
        mentions: [
          {
            name: 'session',
            id: 's_1',
            status: 'resolved',
            summary: 'Chat session "hello" (s_1) · status: done',
            details: 'session::get {"session_id":"s_1"}',
          },
          {
            name: 'session',
            id: 'gone',
            status: 'not-found',
            summary: 'not found: the session worker knows no such id',
          },
        ],
      },
      createdAt: 1,
    })
    // The @ sits where a settled trigger draws its Check: the status slot,
    // with no extra spacer or kind glyph before the text.
    const glyph = container.querySelector('[data-note-glyph="mentions"]')
    expect(glyph?.classList.contains('activity-status-icon')).toBe(true)
    expect(glyph?.querySelector('svg')?.getAttribute('stroke-width')).toBe(
      '2.5',
    )
    expect(container.querySelector('[data-timeline-activity-kind]')).toBeNull()
    expect(
      glyph?.nextElementSibling?.hasAttribute('data-message-summary'),
    ).toBe(true)

    const summary = container.querySelector('[data-message-summary]')
    expect(summary?.textContent).toContain('Context for the agent')
    expect(
      summary?.querySelectorAll('[data-worker-mention="session"]'),
    ).toHaveLength(2)
    expect(container.textContent).not.toContain('<mentions>')

    await expand()
    const detail = container.querySelector('[data-model-note-mentions]')
    expect(detail?.textContent).toContain(
      'Chat session "hello" (s_1) · status: done',
    )
    expect(detail?.textContent).toContain(
      'details call · session::get {"session_id":"s_1"}',
    )
    expect(detail?.textContent).toContain('not found by its worker')
  })
})
