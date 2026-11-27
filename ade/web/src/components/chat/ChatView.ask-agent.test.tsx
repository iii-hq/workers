// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ChatBackend } from '@/lib/backend'
import type { Conversation } from '@/types/chat'

const mocks = vi.hoisted(() => ({
  activateWorkingDir: vi.fn(),
  setDraftText: vi.fn(),
  setDraftAttachments: vi.fn(),
  draftText: undefined as string | undefined,
  ensureSession: undefined as undefined | (() => Promise<void>),
  askAgent: undefined as undefined | ((text: string, send: boolean) => void),
}))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ call: async () => null }),
}))
// Shell present, so the chat checks its folder before it sends.
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
    getDraftText: () => mocks.draftText,
    getDraftAttachments: () => undefined,
    setDraftText: mocks.setDraftText,
    setDraftAttachments: mocks.setDraftAttachments,
    ensureSession: mocks.ensureSession,
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
vi.mock('./Composer', () => ({ Composer: () => <textarea /> }))
vi.mock('./ChatAside', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  // The side panel is what still hands this chat's agent a message.
  ChatAside: (props: { onAskAgent: typeof mocks.askAgent }) => {
    mocks.askAgent = props.onAskAgent
    return null
  },
}))
vi.mock('./MessageList', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  MessageList: () => <div />,
}))

import { ChatView } from './ChatView'

const WORKING_DIR = '/work/project'
const valid = { status: 'valid', path: WORKING_DIR }
const done: Conversation = {
  id: 'root',
  title: 'Fix the parser',
  model: 'anthropic::claude',
  workingDir: WORKING_DIR,
  status: 'done',
  hydrated: true,
  started: true,
  messages: [],
  createdAt: 1,
  updatedAt: 2,
}

let root: Root
let host: HTMLDivElement
const stream = vi.fn<ChatBackend['stream']>(async function* () {})

async function open(conversation: Conversation) {
  const noop = vi.fn()
  await act(async () => {
    root.render(
      <TooltipProvider>
        <ChatView
          key={conversation.id}
          conversation={conversation}
          backend={{ id: 'real', stream } as unknown as ChatBackend}
          modelOptions={[{ id: 'anthropic::claude', label: 'Claude' }]}
          catalogLoading={false}
          onUpdateModel={noop}
          onUpdateThinkingLevel={noop}
          onUpdateWorkingDir={noop}
          onAppendMessage={noop}
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
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  mocks.activateWorkingDir.mockReset().mockResolvedValue(valid)
  mocks.setDraftText.mockReset()
  mocks.setDraftAttachments.mockReset()
  mocks.draftText = undefined
  mocks.ensureSession = undefined
  stream.mockClear()
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.unstubAllGlobals()
})

it('leaves what the side panel asks in the draft while the chat cannot send', async () => {
  mocks.activateWorkingDir.mockReturnValue(new Promise(() => {}))
  await open(done)
  await act(async () => mocks.askAgent?.('Summarize', true))
  expect(stream).not.toHaveBeenCalled()
  expect(mocks.setDraftText).toHaveBeenCalledWith('root', 'Summarize')
})

it('keeps the draft when a send fails before it goes out', async () => {
  mocks.draftText = 'Half a thought'
  mocks.ensureSession = () => Promise.reject(new Error('down'))
  await open(done)
  await act(async () => mocks.askAgent?.('Summarize', true))
  expect(stream).not.toHaveBeenCalled()
  expect(mocks.setDraftText).toHaveBeenCalledExactlyOnceWith(
    'root',
    'Summarize\n\nHalf a thought',
  )
  expect(mocks.setDraftAttachments).not.toHaveBeenCalled()
})
