// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { iiiMentionRuntime, setMentionRuntime } from '@/lib/mentions/runtime'
import type { MentionProvider, MentionView } from '@/lib/mentions/types'
import { registerExtMentionRenderer } from '@/lib/ui-slots'
import type { MentionRendererProps } from '@/types/injectable-ui'
import { MentionPreviewCard } from './MentionPreviewCard'
import { WorkerMentionPill } from './WorkerMentionPill'

vi.mock('@/lib/iii-client', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  getIiiClient: () => new Promise(() => {}),
}))

const kanban: MentionProvider = {
  v: 1,
  name: 'kanban',
  label: 'Tickets',
  icon: 'ticket',
  color: 'blue',
  search: 'kanban::mention::search',
  getFunctionId: 'kanban::mention::get',
}

const views: Record<string, MentionView> = {
  u1: {
    id: 'u1',
    label: 'Fix login redirect',
    hint: 'KAN-12',
    description: 'In progress',
    color: 'rose',
    fields: [
      { label: 'Status', value: 'In progress' },
      { label: 'Priority', value: 'urgent', tone: 'danger' },
    ],
    open: { page: 'kanban-ticket', context: { id: 'KAN-12' } },
    data: { key: 'KAN-12' },
  },
  u2: { id: 'u2', label: 'Second' },
  u3: { id: 'u3', label: 'Third' },
  u4: { id: 'u4', label: 'Fourth' },
}

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  setMentionRuntime({
    listProviders: async () => [kanban],
    search: async () => [],
    get: async (_provider, id) => views[id] ?? null,
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

async function render(node: React.ReactNode) {
  await act(async () => root.render(node))
  for (let i = 0; i < 4; i++) {
    await act(async () => {
      await new Promise<void>((resolve) => setTimeout(resolve, 0))
    })
  }
}

describe('mention preview card', () => {
  it('draws the generic card: provider, handle, name, context, fields', async () => {
    await render(<MentionPreviewCard name="kanban" id="u1" />)
    const card = container.querySelector('[data-mention-card="kanban"]')
    expect(card?.getAttribute('data-color')).toBe('rose')
    expect(card?.textContent).toContain('Tickets')
    expect(card?.textContent).toContain('KAN-12')
    expect(card?.textContent).toContain('Fix login redirect')
    expect(card?.textContent).toContain('In progress')
    expect(card?.querySelector('dd.text-alert')?.textContent).toBe('urgent')
  })

  it('says so when the item is gone', async () => {
    await render(<MentionPreviewCard name="kanban" id="nope" />)
    expect(container.textContent).toContain('this item no longer exists')
  })

  it("uses the worker's renderer, and the generic card when it throws", async () => {
    const seen: MentionRendererProps[] = []
    const off = registerExtMentionRenderer({
      provider: 'kanban',
      scope: 'kanban',
      path: 'kanban/page.js',
      Preview: (props) => {
        seen.push(props)
        return (
          <div data-custom-preview>
            {String((props.view.data as { key: string }).key)}
          </div>
        )
      },
    })
    await render(<MentionPreviewCard name="kanban" id="u1" />)
    expect(container.querySelector('[data-custom-preview]')?.textContent).toBe(
      'KAN-12',
    )
    expect(container.querySelector('[data-iii-ui="kanban"]')).not.toBeNull()
    expect(seen.at(-1)?.provider).toBe('kanban')
    expect(seen.at(-1)?.view.id).toBe('u1')
    off()

    const silence = vi.spyOn(console, 'error').mockImplementation(() => {})
    const offBroken = registerExtMentionRenderer({
      provider: 'kanban',
      scope: 'kanban',
      path: 'kanban/page.js',
      Preview: () => {
        throw new Error('broken renderer')
      },
    })
    await render(<MentionPreviewCard name="kanban" id="u1" />)
    expect(
      container.querySelector('[data-mention-card="kanban"]'),
    ).not.toBeNull()
    offBroken()
    silence.mockRestore()
  })
})

describe('mention pill', () => {
  it('previews on hover once resolved, unless told not to', async () => {
    await render(<WorkerMentionPill name="kanban" id="u1" openOnClick />)
    const pill = container.querySelector('[data-worker-mention="kanban"]')
    expect(pill?.textContent).toContain('Fix login redirect')
    // The pill is the trigger of the hover popover that holds the card.
    expect(pill?.getAttribute('data-state')).toBe('closed')

    await render(<WorkerMentionPill name="kanban" id="u1" hoverCard={false} />)
    const plain = container.querySelector('[data-worker-mention="kanban"]')
    expect(plain?.hasAttribute('data-state')).toBe(false)
  })
})
