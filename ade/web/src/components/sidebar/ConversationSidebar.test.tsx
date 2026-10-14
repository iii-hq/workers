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
          onRename={vi.fn()}
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
          onRename={vi.fn()}
          onRemove={vi.fn()}
        />
      </TooltipProvider>,
    )
    expect(html).toContain('No conversations match these filters.')
    expect(html).toContain('Clear filters')
  })
})

describe('ConversationSidebar drafts', () => {
  it('leaves an unsent new chat off the list until its first send', () => {
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <ConversationSidebar
          conversations={[
            { ...conversation('draft', 'new chat'), draft: true },
          ]}
          activeId="draft"
          onSelect={vi.fn()}
          onRename={vi.fn()}
          onRemove={vi.fn()}
        />
      </TooltipProvider>,
    )
    expect(html).not.toContain('open new chat')
    expect(html).toContain('No conversations yet.')
  })
})

describe('ConversationSidebar side chats', () => {
  it('hides a side chat and what it spawned until it is kept', () => {
    const list: Conversation[] = [
      conversation('main', 'Main chat'),
      {
        ...conversation('side', 'Side chat · Main chat'),
        sessionMetadata: { side_chat_of: 'main' },
      },
      { ...conversation('helper', 'Helper inside side'), parentId: 'side' },
    ]
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <ConversationSidebar
          conversations={list}
          activeId={null}
          onSelect={vi.fn()}
          onRename={vi.fn()}
          onRemove={vi.fn()}
        />
      </TooltipProvider>,
    )
    expect(html).toContain('aria-label="open Main chat"')
    expect(html).not.toContain('Side chat · Main chat')
    expect(html).not.toContain('Helper inside side')
  })
})
