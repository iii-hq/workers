import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ChatBackend } from '@/lib/backend'
import { sideChatQuote } from '@/lib/chat-aside'
import type { Conversation } from '@/types/chat'
import { ChatView } from './ChatView'

const fork: Conversation = {
  id: 'fork',
  title: 'Side chat · Main',
  model: null,
  hydrated: true,
  workingDir: '/work/repo',
  messages: [
    { id: 'copied', role: 'user', content: 'Copied from main', createdAt: 5 },
    { id: 'own', role: 'user', content: 'Asked on the side', createdAt: 30 },
  ],
  createdAt: 20,
  updatedAt: 30,
  sessionMetadata: { side_chat_of: 'main' },
}

function render(
  conversation: Conversation,
  side?: React.ComponentProps<typeof ChatView>['side'],
) {
  const mutate = vi.fn()
  return renderToStaticMarkup(
    <TooltipProvider>
      <ChatView
        conversation={conversation}
        backend={{ id: 'mock' } as ChatBackend}
        modelOptions={[]}
        catalogLoading={false}
        onUpdateModel={mutate}
        onUpdateThinkingLevel={mutate}
        onUpdateWorkingDir={mutate}
        onAppendMessage={mutate}
        onPatchMessage={mutate}
        onCompactConversation={mutate}
        side={side}
      />
    </TooltipProvider>,
  )
}

describe('ChatView as a side chat', () => {
  it('shows only what came after the fork, with no page header', () => {
    const html = render(fork, { since: 20, empty: <p>EMPTY</p> })
    expect(html).toContain('Asked on the side')
    expect(html).not.toContain('Copied from main')
    // The page header's status read-out (a message has its own <header>).
    expect(html).not.toContain('class="sr-only">ready</span>')
    expect(html).not.toContain('EMPTY')
    expect(html).toContain('contentEditable="true"')
  })

  it('shows its hint until the side chat has messages', () => {
    const html = render(
      { ...fork, messages: fork.messages.slice(0, 1) },
      { since: 20, empty: <p>EMPTY</p> },
    )
    expect(html).toContain('EMPTY')
    expect(html).not.toContain('Copied from main')
  })

  it('leaves a normal chat whole, header included', () => {
    const html = render(fork)
    expect(html).toContain('Copied from main')
    expect(html).toContain('class="sr-only">ready</span>')
  })
})

describe('ChatView side chat quotes', () => {
  const quote = {
    messageId: 'source',
    text: 'The views stay local.',
    agent: 'Frontend Engineer',
  }

  it('shows the pending quote at the top of the composer, removable', () => {
    const html = render(fork, {
      since: 20,
      empty: null,
      quote,
      onRemoveQuote: vi.fn(),
    })
    expect(html).toContain('data-side-chat-quote')
    expect(html).toContain('Quoted · Frontend Engineer')
    expect(html).toContain('aria-label="Remove quote"')
  })

  it('turns a quoted question into a card and the question', () => {
    const asked: Conversation = {
      ...fork,
      messages: [
        {
          id: 'asked',
          role: 'user',
          content: `${sideChatQuote(quote.text)}Do they sync?`,
          createdAt: 30,
        },
      ],
    }
    const html = render(asked, { since: 20, empty: null, quotes: [quote] })
    expect(html).toContain('aria-label="Show the quoted message"')
    expect(html).toContain('Quoted · Frontend Engineer')
    expect(html).toContain('Do they sync?')
    expect(html).not.toContain('<blockquote')
  })

  it('leaves a blockquote it did not send as plain markdown', () => {
    const asked: Conversation = {
      ...fork,
      messages: [
        {
          id: 'asked',
          role: 'user',
          content: '> someone else\n\nhi',
          createdAt: 30,
        },
      ],
    }
    const html = render(asked, { since: 20, empty: null, quotes: [quote] })
    expect(html).not.toContain('Show the quoted message')
    expect(html).toContain('<blockquote')
  })
})
