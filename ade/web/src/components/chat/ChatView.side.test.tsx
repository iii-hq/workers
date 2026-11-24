import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ChatBackend } from '@/lib/backend'
import { SIDE_CHAT_SUMMARY_PROMPT, sideChatQuote } from '@/lib/chat-aside'
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

/** The attributes of the one element with `role="textbox"`, as a screen reader finds it. */
function textbox(html: string): Record<string, string> | null {
  const tag = html.match(/<div[^>]*role="textbox"[^>]*>/)?.[0]
  if (!tag) return null
  return Object.fromEntries(
    [...tag.matchAll(/([\w-]+)="([^"]*)"/g)].map(([, key, value]) => [
      key,
      value,
    ]),
  )
}

describe('ChatView as an engine side chat (chat-projects §2.3.2)', () => {
  const engineSide = (
    extra: Partial<NonNullable<React.ComponentProps<typeof ChatView>['side']>>,
  ) => ({
    since: 20,
    empty: null,
    locked: {
      label: 'Claude Opus 5.5 · High',
      model: 'anthropic::claude-opus-5-5',
    },
    placeholder: 'Ask aside…',
    metaLine: 'Claude Opus 5.5 · High · Work',
    onBringBack: vi.fn(),
    bringBackLabel: 'Bring back',
    ...extra,
  })

  it('shows the locked model label instead of the picker, the meta line and "Ask aside…"', () => {
    const html = render(fork, engineSide({}))
    expect(html).toContain('data-locked-model')
    expect(html).toContain('aria-label="Claude Opus 5.5 · High, locked"')
    expect(html).toContain('data-side-chat-meta')
    expect(html).toContain('Claude Opus 5.5 · High · Work')
    expect(html).toContain('Ask aside…')
    // No model picker: its trigger names the (absent) model.
    expect(html).not.toContain('No model')
    expect(render(fork, { since: 20, empty: null })).toContain('No model')
  })

  it('names its textbox "Ask aside…" for screen readers; the main composer keeps its own name', () => {
    const side = textbox(render(fork, engineSide({})))
    expect(side).toMatchObject({
      'aria-label': 'Ask aside…',
      'aria-placeholder': 'Ask aside…',
    })
    const main = textbox(render(fork))
    expect(main?.['aria-label']).toBe('message composer')
    expect(main?.['aria-placeholder']).toBeTruthy()
  })

  it('draws the summary prompt as one line and its reply as the summary card', () => {
    const summarized: Conversation = {
      ...fork,
      messages: [
        {
          id: 'msg-1-user-0',
          role: 'user',
          content: SIDE_CHAT_SUMMARY_PROMPT,
          createdAt: 30,
        },
        {
          id: 'e-sum',
          role: 'assistant',
          content: 'Keep five fast attempts, then the dead-letter table.',
          createdAt: 31,
        },
      ],
    }
    const html = render(summarized, engineSide({ summaries: ['e-sum'] }))
    expect(html).toContain('Asked for a summary')
    expect(html).not.toContain('in at most five short lines')
    expect(html).toContain('data-side-chat-summary')
    expect(html).toContain('Summary for the main chat')
    expect(html).toContain('Keep five fast attempts')
    expect(html).toContain('data-bring-back')
    expect(html).toContain('>Copy<')
  })

  it('offers "Bring back" on every agent reply, labelled for a phone too', () => {
    const replied: Conversation = {
      ...fork,
      messages: [
        { id: 'own', role: 'user', content: 'Why five?', createdAt: 30 },
        {
          id: 'reply',
          role: 'assistant',
          content: 'Eight outlasts the receipt.',
          createdAt: 31,
        },
      ],
    }
    expect(render(replied, engineSide({}))).toContain('Bring back')
    expect(
      render(
        replied,
        engineSide({ bringBackLabel: 'Bring back to Webhook retries' }),
      ),
    ).toContain('Bring back to Webhook retries')
    // A side chat of the ADE's own fork has no "Bring back".
    expect(render(replied, { since: 20, empty: null })).not.toContain(
      'data-bring-back',
    )
  })
})
