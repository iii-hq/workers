import { renderToStaticMarkup } from 'react-dom/server'
import { beforeEach, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ChatBackend } from '@/lib/backend'
import { realBackend } from '@/lib/backend/real'
import type { Conversation } from '@/types/chat'
import { ChatView } from './ChatView'
import type { ComposerSubmitPayload } from './Composer'

const trigger = vi.hoisted(() => vi.fn())
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ trigger }),
}))
vi.mock('@/lib/conversations-context', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  useConversationsCtxOptional: () => null,
}))
const composer = vi.hoisted(() => ({
  props: null as null | {
    onSubmit: (payload: ComposerSubmitPayload) => Promise<void>
  },
}))
vi.mock('./Composer', () => ({
  Composer: (props: NonNullable<typeof composer.props>) => {
    composer.props = props
    return <textarea />
  },
}))

const parent: Conversation = {
  id: 'console-parent',
  title: 'Observe spec',
  model: 'anthropic::claude-opus-5-5',
  hydrated: true,
  started: true,
  messages: [
    {
      id: 'e_reply',
      role: 'assistant',
      content: 'Round 2 sent to the executor.',
      createdAt: 1,
    },
  ],
  createdAt: 1,
  updatedAt: 2,
}

const onAppendMessage = vi.fn()
const onPatchMessage = vi.fn()
const onCompactConversation = vi.fn()

function submit(text: string, backend: ChatBackend = realBackend) {
  renderToStaticMarkup(
    <TooltipProvider>
      <ChatView
        conversation={parent}
        backend={backend}
        modelOptions={[
          {
            id: 'anthropic::claude-opus-5-5',
            label: 'Opus 5.5',
            contextWindow: 200_000,
          },
        ]}
        catalogLoading={false}
        onUpdateModel={vi.fn()}
        onUpdateThinkingLevel={vi.fn()}
        onUpdateWorkingDir={vi.fn()}
        onAppendMessage={onAppendMessage}
        onPatchMessage={onPatchMessage}
        onCompactConversation={onCompactConversation}
      />
    </TooltipProvider>,
  )
  if (!composer.props) throw new Error('composer did not mount')
  return composer.props.onSubmit({ text, attachments: [] })
}

const appended = () => onAppendMessage.mock.calls.map(([, message]) => message)

beforeEach(() => {
  composer.props = null
  onAppendMessage.mockReset()
  onPatchMessage.mockReset()
  onCompactConversation.mockReset()
  trigger.mockReset()
})

// The investigated parent spent most of the night waiting on sub-agents or
// waking up to their notifications. `/compact` used to be refused there as
// "another compaction is in progress"; now the harness takes the command
// whatever the turn is doing and reports to every client.
it('/compact goes to the harness as a send, with no local verdict', async () => {
  trigger.mockResolvedValue({
    session_id: parent.id,
    turn_id: 't-parent',
    accepted: true,
    compaction: { compaction_id: 'cmp_1', entry_id: 'e_idem_x' },
  })

  await submit('/compact keep the ticket ids')

  const sends = trigger.mock.calls.filter(([id]) => id === 'harness::send')
  expect(sends).toHaveLength(1)
  const [, payload] = sends[0]
  expect(payload).toMatchObject({
    session_id: parent.id,
    message: '/compact keep the ticket ids',
    model: 'claude-opus-5-5',
    provider: 'anthropic',
  })
  // The optimistic row carries the send's entry id: the harness's start
  // notice takes it, on this device and every other.
  const [row] = appended()
  expect(row).toMatchObject({ role: 'user', command: true })
  expect(row.id).toBe(`e_idem_${payload.idempotency_key}`)
  // No local "compacting…" notice and no local verdict: both come from the
  // server, so every device shows the same thing.
  expect(appended()).toHaveLength(1)
  expect(onPatchMessage).not.toHaveBeenCalled()
  expect(trigger.mock.calls.map(([id]) => id)).toEqual(['harness::send'])
})

it('a send the harness refuses becomes an error notice', async () => {
  trigger.mockRejectedValue(new Error('harness/invalid_request: read-only'))

  await submit('/compact')

  expect(appended()[1]).toMatchObject({
    role: 'system',
    tone: 'error',
    content: expect.stringContaining('read-only'),
  })
})

it('a local backend still compacts its own transcript', async () => {
  const compactSession = vi.fn(async () => ({
    status: 'ok' as const,
    tokensBefore: 1_200,
    autoContinued: false,
    summaryText: 'S',
    compactionEntryId: 'm',
  }))
  await submit('/compact', {
    id: 'mock',
    compactSession,
  } as unknown as ChatBackend)

  expect(compactSession).toHaveBeenCalledWith(
    parent.id,
    parent.model,
    200_000,
    undefined,
  )
  expect(onCompactConversation).toHaveBeenCalledWith(
    parent.id,
    expect.objectContaining({ kind: 'compaction', tokensBefore: 1_200 }),
  )
})
