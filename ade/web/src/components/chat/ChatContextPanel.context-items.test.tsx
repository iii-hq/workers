// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { parseContextItems } from '@/lib/backend/context-items'
import { resetFoldedGroups } from '@/lib/context-groups'
import type { Conversation, Message } from '@/types/chat'
;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mocks = vi.hoisted(() => ({ call: vi.fn() }))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: () => Promise.resolve({ call: mocks.call }),
}))
vi.mock('@/lib/panel-context', () => ({ requestPanelOpen: vi.fn() }))

import { ChatContextPanel } from './ChatContextPanel'

const items = parseContextItems({
  session_id: 'main',
  message_count: 400,
  generated_at_ms: 1,
  subagents: [],
  pull_requests: [
    {
      repo: 'acme/web',
      number: 482,
      url: 'https://github.com/acme/web/pull/482',
      title: 'Retry webhook deliveries',
      state: 'merged',
      checks: 'passing',
      entry_id: 'e-pr',
      source: 'created',
    },
  ],
  links: [
    {
      url: 'https://docs.example.com/webhooks/retries',
      host: 'docs.example.com',
      title: 'Webhook retry guidance',
      entry_id: 'e-old',
      count: 1,
    },
  ],
  attachments: [
    {
      attachment_id: 'att-1',
      name: 'delivery-log.txt',
      mime: 'text/plain',
      size: 49_152,
      direction: 'sent',
      entry_id: 'e-att',
    },
  ],
  images: [
    {
      attachment_id: 'img-1',
      mime: 'image/png',
      direction: 'sent',
      entry_id: 'e-i1',
      block_index: 0,
    },
    {
      attachment_id: 'img-2',
      mime: 'image/png',
      direction: 'received',
      entry_id: 'e-i2',
      block_index: 0,
    },
  ],
  side_chats: [],
  truncated: {},
})

function chat(extra: Partial<Conversation> = {}): Conversation {
  return {
    id: 'main',
    title: 'main',
    model: null,
    messages: [],
    createdAt: 1,
    updatedAt: 1,
    ...extra,
  }
}

let root: Root | null = null
let container: HTMLElement | null = null

async function render(
  conversation: Conversation,
  extra: Partial<Parameters<typeof ChatContextPanel>[0]> = {},
) {
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
  await act(async () => {
    root?.render(
      <TooltipProvider delayDuration={0}>
        <ChatContextPanel
          conversation={conversation}
          conversations={[conversation]}
          connectionState="connected"
          observers={[]}
          onDisposeObserver={vi.fn()}
          onOpenConversation={vi.fn()}
          shellAvailable={false}
          githubAvailable={false}
          {...extra}
        />
      </TooltipProvider>,
    )
  })
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
  return container
}

const sectionLabels = (view: HTMLElement) =>
  [...view.querySelectorAll('section[data-reorder-item]')]
    .filter((s) => !(s as HTMLElement).hidden)
    .map((s) => s.getAttribute('aria-label'))

beforeEach(() => {
  window.localStorage.clear()
  resetFoldedGroups()
  mocks.call.mockReset()
  mocks.call.mockImplementation((fn: string) =>
    fn === 'ade::context::list'
      ? Promise.resolve({ items: [] })
      : new Promise(() => {}),
  )
})

afterEach(() => {
  act(() => root?.unmount())
  container?.remove()
})

describe('the context pane reading harness::context-items (D10, D11)', () => {
  it('lists what the engine gathered, beyond the loaded transcript, common sections first', async () => {
    // Nothing loaded mentions these: they come from the engine's lists.
    const view = await render(chat(), { items })
    expect(sectionLabels(view)).toEqual([
      'Pull requests',
      'Links',
      'Attachments',
      'Images',
      'Tracking',
      'Project',
    ])
    const text = view.textContent ?? ''
    expect(text).toContain('#482')
    expect(text).toContain('Retry webhook deliveries')
    expect(text).toContain('Webhook retry guidance')
    expect(text).toContain('https://docs.example.com/webhooks/retries')
    expect(text).toContain('delivery-log.txt')
    expect(text).toContain('48kb')
    expect(text).toContain('Sent')
    expect(
      view.querySelectorAll('[data-image-grid] [data-direction]'),
    ).toHaveLength(2)
    expect(view.querySelector('[data-direction="received"]')).not.toBeNull()
  })

  it('falls back to its own parse of the loaded transcript without them', async () => {
    const say: Message = {
      id: 'e-1:0',
      role: 'assistant',
      content:
        'see https://example.com/local and https://github.com/o/r/pull/7',
      createdAt: 1,
    }
    const view = await render(chat({ messages: [say] }), { items: null })
    const text = view.textContent ?? ''
    expect(text).toContain('example.com')
    expect(text).not.toContain('Webhook retry guidance')
    expect(sectionLabels(view)).toContain('Links')
  })

  it('draws a pull request the engine could not read (no github worker) without a state', async () => {
    const view = await render(chat(), {
      items: parseContextItems({
        session_id: 'main',
        pull_requests: [
          {
            repo: 'acme/web',
            number: 483,
            url: 'https://github.com/acme/web/pull/483',
            entry_id: 'e-1',
            source: 'created',
          },
        ],
      }),
    })
    const text = view.textContent ?? ''
    expect(text).toContain('#483')
    expect(text).toContain('Created here')
    expect(view.querySelector('[data-pr-state]')).toBeNull()
    expect(text).not.toContain('Open')
  })

  it("keeps the engine's pull requests when gh in the folder reads none", async () => {
    // gh runs (the shell worker is there) but never answers for these PRs:
    // the engine's rows still list them.
    const view = await render(chat({ workingDir: '/work/acme' }), {
      items,
      shellAvailable: true,
      githubAvailable: true,
    })
    expect(view.textContent).toContain('#482')
    expect(
      view.querySelector('section[aria-label="Pull requests"]'),
    ).not.toBeNull()
  })

  it('says when nothing was gathered yet', async () => {
    const view = await render(chat(), {
      items: parseContextItems({ session_id: 'main' }),
    })
    expect(view.textContent).toContain('Nothing gathered yet')
    expect(view.textContent).toContain(
      'Links, files, images, subagents and pull requests from this conversation show up here.',
    )
  })

  it('offers Show in conversation with the entry, for an entry far back', async () => {
    const onShow = vi.fn()
    const view = await render(chat(), {
      items,
      onShowInConversation: onShow,
    })
    const trigger = view.querySelector(
      'button[aria-label="Actions for https://docs.example.com/webhooks/retries"]',
    ) as HTMLButtonElement
    expect(trigger).not.toBeNull()
    await act(async () => {
      trigger.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
      trigger.click()
    })
    const item = [...document.querySelectorAll('[role="menuitem"]')].find(
      (el) => el.textContent?.includes('Show in conversation'),
    ) as HTMLElement | undefined
    expect(item).toBeDefined()
    await act(async () => {
      item?.click()
    })
    expect(onShow).toHaveBeenCalledWith({
      entryId: 'e-old',
      messageId: undefined,
    })
  })

  it('filters to one section from a phone chip', async () => {
    const view = await render(chat(), { items, phone: true })
    const chips = [...view.querySelectorAll('[role="toolbar"] button')].map(
      (b) => b.textContent,
    )
    // Empty sections have no chip.
    expect(chips).toEqual(['All', 'PRs1', 'Links1', 'Files1', 'Images2'])
    await act(async () => {
      ;(
        [...view.querySelectorAll('[role="toolbar"] button')].find(
          (b) => b.textContent === 'Links1',
        ) as HTMLButtonElement
      ).click()
    })
    expect(sectionLabels(view)).toEqual(['Links'])
  })
})
