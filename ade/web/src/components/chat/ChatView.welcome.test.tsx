import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ChatBackend } from '@/lib/backend'
import type { Conversation } from '@/types/chat'
import { ChatView } from './ChatView'

const base: Conversation = {
  id: 'c1',
  title: 'New chat',
  model: null,
  hydrated: true,
  messages: [],
  createdAt: 1,
  updatedAt: 1,
}

function render(session: Conversation) {
  const mutate = vi.fn()
  return renderToStaticMarkup(
    <TooltipProvider>
      <ChatView
        conversation={session}
        backend={{ id: 'mock' } as ChatBackend}
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

describe('the welcome screen', () => {
  it('greets a draft chat', () => {
    expect(render({ ...base, draft: true })).toContain('data-welcome="true"')
  })

  it('greets an opened session that never started', () => {
    expect(render(base)).toContain('data-welcome="true"')
  })

  it("never greets a sub-agent's session, even with no transcript loaded", () => {
    const spawned: Conversation = {
      ...base,
      id: 's_child',
      title: 'Film name: iii-pocket-teaser',
      parentId: 'console-parent',
    }
    expect(render(spawned)).not.toContain('data-welcome="true"')
    expect(
      render({
        ...base,
        spawnedBy: { kind: 'harness', name: 'Helper' } as never,
      }),
    ).not.toContain('data-welcome="true"')
  })

  it('never greets a session that has started', () => {
    expect(render({ ...base, started: true })).not.toContain(
      'data-welcome="true"',
    )
  })
})
