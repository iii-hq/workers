// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ChatBackend } from '@/lib/backend'
import type { Conversation } from '@/types/chat'

const mocks = vi.hoisted(() => ({
  call: vi.fn(),
  activateWorkingDir: vi.fn(),
  composerWorkingDir: undefined as string | null | undefined,
  composerModel: undefined as string | null | undefined,
  composerSubmit: undefined as
    | undefined
    | ((payload: { text: string; attachments: [] }) => Promise<void>),
  stream: vi.fn(async function* () {}),
  setDraftText: vi.fn(),
  setDraftAttachments: vi.fn(),
}))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ call: mocks.call }),
}))
// Shell present, so the composer's project chip and the open-time
// reconcile are live, as in the real app.
vi.mock('@/lib/conversations-context', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  useConversationsCtxOptional: () => ({
    conversations: [],
    connectionState: 'connected',
    harnessStatus: {
      present: true,
      loading: false,
      installing: false,
      error: null,
    },
    getDraftText: () => undefined,
    getDraftAttachments: () => undefined,
    setDraftText: mocks.setDraftText,
    setDraftAttachments: mocks.setDraftAttachments,
    shellAvailable: true,
    worktreeAvailable: false,
    memoryAvailable: false,
    approvalGateAvailable: false,
  }),
}))
vi.mock('@/lib/working-dir', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  activateWorkingDir: mocks.activateWorkingDir,
}))
vi.mock('./Composer', () => ({
  Composer: (props: {
    workingDir?: string | null
    model?: string | null
    onSubmit?: typeof mocks.composerSubmit
  }) => {
    mocks.composerWorkingDir = props.workingDir
    mocks.composerModel = props.model
    mocks.composerSubmit = props.onSubmit
    return <textarea />
  },
}))

vi.mock('./ChatAside', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  ChatAside: () => null,
}))
vi.mock('./MessageList', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  MessageList: () => <div />,
}))

import { ChatView } from './ChatView'

const WORKTREE = '/work/project/.wt/kan-8'

function conversation(parentId?: string): Conversation {
  return {
    id: parentId ? 'child' : 'root',
    title: 'KAN-8 · case against',
    model: 'anthropic::claude',
    workingDir: WORKTREE,
    parentId,
    hydrated: true,
    started: true,
    messages: [],
    createdAt: 1,
    updatedAt: 2,
  }
}

let root: Root
let host: HTMLDivElement
const onUpdateWorkingDir = vi.fn()
const onAppendMessage = vi.fn()

async function open(
  session: Conversation,
  modelOptions = [{ id: 'anthropic::claude', label: 'Claude' }],
) {
  const noop = vi.fn()
  await act(async () => {
    root.render(
      <TooltipProvider>
        <ChatView
          conversation={session}
          backend={
            { id: 'real', stream: mocks.stream } as unknown as ChatBackend
          }
          modelOptions={modelOptions}
          catalogLoading={false}
          onUpdateModel={noop}
          onUpdateThinkingLevel={noop}
          onUpdateWorkingDir={onUpdateWorkingDir}
          onAppendMessage={onAppendMessage}
          onPatchMessage={noop}
          onCompactConversation={noop}
        />
      </TooltipProvider>,
    )
  })
  // Let the activation promise settle.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
}

beforeEach(() => {
  ;(
    globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true
  mocks.call.mockReset().mockResolvedValue(null)
  // The run's worktree is gone: activation would fall back to the default.
  mocks.activateWorkingDir
    .mockReset()
    .mockResolvedValue({ status: 'recovered', path: '/home/default' })
  mocks.composerWorkingDir = undefined
  mocks.composerModel = undefined
  mocks.composerSubmit = undefined
  mocks.stream.mockClear()
  onUpdateWorkingDir.mockReset()
  onAppendMessage.mockReset()
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})

afterEach(() => {
  act(() => root.unmount())
  host.remove()
})

it('shows a sub-agent its own project and never moves it when the folder is gone', async () => {
  await open(conversation('parent'))
  expect(mocks.composerWorkingDir).toBe(WORKTREE)
  expect(mocks.activateWorkingDir).not.toHaveBeenCalled()
  expect(onUpdateWorkingDir).not.toHaveBeenCalled()
  expect(onAppendMessage).not.toHaveBeenCalled()
})

it('still recovers a root chat whose folder is gone', async () => {
  await open(conversation())
  expect(mocks.activateWorkingDir).toHaveBeenCalledWith(WORKTREE)
  expect(onUpdateWorkingDir).toHaveBeenCalledWith('root', '/home/default')
})

it('shows a profile plain router model as its catalog entry', async () => {
  // Agent profiles may name `claude-opus-5-5` rather than a catalog key; the
  // picker only selects catalog ids, so it read "Choose model".
  await open({ ...conversation('parent'), model: 'claude-opus-5-5' }, [
    { id: 'anthropic::claude-opus-5-5', label: 'Opus 5.5' },
  ])
  expect(mocks.composerModel).toBe('anthropic::claude-opus-5-5')
})

it('does not send a sub-agent whose folder is gone; it asks for a folder', async () => {
  await open(conversation('parent'))
  await act(async () => {
    await mocks.composerSubmit?.({ text: 'continue', attachments: [] })
  })
  expect(mocks.stream).not.toHaveBeenCalled()
  expect(onUpdateWorkingDir).not.toHaveBeenCalled()
  const notice = onAppendMessage.mock.calls.at(-1)?.[1] as { content: string }
  expect(notice.content).toContain(WORKTREE)
  expect(notice.content).toContain('Pick a folder')
  // The text is not lost: it goes back into the composer.
  expect(mocks.setDraftText).toHaveBeenCalledWith('child', 'continue')
})

it('sends a sub-agent whose folder still exists, in that folder', async () => {
  mocks.activateWorkingDir.mockResolvedValue({
    status: 'valid',
    path: WORKTREE,
  })
  await open(conversation('parent'))
  await act(async () => {
    await mocks.composerSubmit?.({ text: 'continue', attachments: [] })
  })
  expect(mocks.stream).toHaveBeenCalledTimes(1)
  expect(mocks.stream.mock.calls[0]?.[2]).toMatchObject({
    workingDir: WORKTREE,
  })
})

it('sends a profile plain router model as its catalog key', async () => {
  // The backend guesses a provider for ids that are not catalog keys
  // (`claude…` → anthropic), which failed `claude-code/…` profile models.
  mocks.activateWorkingDir.mockResolvedValue({
    status: 'valid',
    path: WORKTREE,
  })
  await open(
    { ...conversation('parent'), model: 'claude-code/claude-haiku-5-5' },
    [{ id: 'claude-code::claude-code/claude-haiku-5-5', label: 'Haiku 5.5' }],
  )
  await act(async () => {
    await mocks.composerSubmit?.({ text: 'pong?', attachments: [] })
  })
  expect(mocks.stream.mock.calls[0]?.[1]).toBe(
    'claude-code::claude-code/claude-haiku-5-5',
  )
})
