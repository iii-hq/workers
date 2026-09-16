import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { Conversation } from '@/types/chat'
import { ConversationSidebar } from './ConversationSidebar'

function conversation(id: string, title: string): Conversation {
  return {
    id,
    title,
    model: null,
    messages: [],
    hydrated: true,
    createdAt: 1,
    updatedAt: 1,
  }
}

describe('ConversationSidebar rename affordance', () => {
  it('gives every row a named rename action, not just double-click and F2', () => {
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <ConversationSidebar
          conversations={[conversation('c1', 'Frontend')]}
          activeId="c1"
          narrow
          onSelect={vi.fn()}
          onRename={vi.fn(async () => null)}
          onRemove={vi.fn()}
        />
      </TooltipProvider>,
    )

    // The palette command reaches the open chat's editor through exactly this
    // pair of attributes: `[aria-current="page"] [data-conversation-rename]`.
    expect(html).toContain('aria-current="page"')
    expect(html).toContain('data-conversation-rename=""')
    expect(html).toContain('aria-label="rename Frontend"')
  })
})

describe('ConversationSidebar kind filter', () => {
  const list: Conversation[] = [
    conversation('chat', 'Frontend'),
    { ...conversation('suite', 'Shell coder sandbox'), kind: 'e2e' },
    { ...conversation('suite-worker', 'Probe runner'), parentId: 'suite' },
  ]

  it('lists only user chats by default and hides an e2e subtree whole', () => {
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <ConversationSidebar
          conversations={list}
          activeId={null}
          onSelect={vi.fn()}
          onRename={vi.fn(async () => null)}
          onRemove={vi.fn()}
        />
      </TooltipProvider>,
    )
    expect(html).toContain('aria-label="open Frontend"')
    expect(html).not.toContain('Shell coder sandbox')
    expect(html).not.toContain('Probe runner')
    /* Nothing differs from the defaults yet, so the trigger stays quiet. */
    expect(html).not.toContain('data-filtering')
  })

  it('names the cause when the filter hides every chat', () => {
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <ConversationSidebar
          conversations={[{ ...conversation('suite', 'Suite'), kind: 'e2e' }]}
          activeId={null}
          onSelect={vi.fn()}
          onRename={vi.fn(async () => null)}
          onRemove={vi.fn()}
        />
      </TooltipProvider>,
    )
    expect(html).toContain('No conversations match these filters.')
    expect(html).toContain('Clear filters')
  })
})
