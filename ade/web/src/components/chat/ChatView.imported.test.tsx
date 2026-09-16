import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ChatBackend } from '@/lib/backend'
import type { Conversation } from '@/types/chat'
import { ChatView } from './ChatView'

const conversation: Conversation = {
  id: 'imported',
  title: 'Imported',
  model: null,
  hydrated: true,
  messages: [
    {
      id: 'original-message',
      role: 'user',
      content: 'Original conversation text',
      createdAt: 1,
    },
  ],
  createdAt: 1,
  updatedAt: 2,
  sessionMetadata: {
    external_source: 'codex',
    external_session_id: 'original',
    source_cwd: '/source/project',
  },
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
it('renders a normal composer for a native imported conversation', () => {
  const html = render(conversation)
  expect(html).toContain('Original conversation text')
  expect(html).toContain('contentEditable="true"')
  expect(html).toContain('Choose an ADE model and working directory')
  expect(html).not.toContain('Update history')
  expect(html).not.toContain('Read-only')
  expect(html).not.toContain('/source/project')
})
it('preserves the generic read-only session guard', () => {
  const html = render({ ...conversation, sessionMetadata: { read_only: true } })
  expect(html).toContain('This conversation is read-only.')
  expect(html).not.toContain('contentEditable="true"')
  expect(html).not.toContain('Update history')
})
