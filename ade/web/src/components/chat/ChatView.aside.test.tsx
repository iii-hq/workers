import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ChatBackend } from '@/lib/backend'
import type { Conversation } from '@/types/chat'

vi.mock('./ChatAside', () => ({
  ChatAside: () => <div data-aside="" />,
  QuoteChip: () => null,
  SelectionToolbar: () => <div data-selection-toolbar="" />,
}))
vi.mock('@/lib/conversations-context', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  useConversationsCtxOptional: () => ({
    conversations: [],
    connectionState: 'connected',
    harnessStatus: { kind: 'ready' },
    getDraftText: () => undefined,
    getDraftAttachments: () => undefined,
    shellAvailable: true,
    worktreeAvailable: false,
    approvalGateAvailable: false,
  }),
}))

import { ChatView } from './ChatView'

const fresh: Conversation = {
  id: 'new',
  title: 'New chat',
  model: null,
  hydrated: true,
  messages: [],
  createdAt: 1,
  updatedAt: 1,
}

function render(conversation: Conversation) {
  const mutate = vi.fn()
  return renderToStaticMarkup(
    <TooltipProvider>
      <ChatView
        conversation={conversation}
        backend={{ id: 'real' } as ChatBackend}
        modelOptions={[]}
        catalogLoading={false}
        onUpdateModel={mutate}
        onUpdateThinkingLevel={mutate}
        onUpdateWorkingDir={mutate}
        onAppendMessage={mutate}
        onPatchMessage={mutate}
        onCompactConversation={mutate}
      />
    </TooltipProvider>,
  )
}

describe('ChatView side panel', () => {
  it('offers no side panel before the chat starts', () => {
    const html = render(fresh)
    expect(html).not.toContain('data-aside')
    expect(html).not.toContain('data-selection-toolbar')
  })

  it('offers it once the chat has started', () => {
    expect(render({ ...fresh, started: true })).toContain('data-aside')
    expect(
      render({
        ...fresh,
        messages: [{ id: 'm', role: 'user', content: 'hi', createdAt: 2 }],
      }),
    ).toContain('data-aside')
  })
})
