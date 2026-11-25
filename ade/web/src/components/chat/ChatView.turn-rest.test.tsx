// @vitest-environment jsdom
import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { DESKTOP_POINTER_QUERY } from '@/hooks/use-media-query'
import type { ChatBackend } from '@/lib/backend'
import { onComposerFocusRequest } from '@/lib/composer-insert'
import type { Conversation } from '@/types/chat'

const mocks = vi.hoisted(() => ({
  activateWorkingDir: vi.fn(),
  setDraftText: vi.fn(),
  setDraftAttachments: vi.fn(),
  draftText: undefined as string | undefined,
  ensureSession: undefined as undefined | (() => Promise<void>),
  composerSubmit: undefined as
    | undefined
    | ((payload: { text: string; attachments: [] }) => Promise<void>),
}))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ call: async () => null }),
}))
// Shell present, so a send checks the folder first: that await is the gap
// between the click and the chat turning working.
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
vi.mock('./Composer', () => ({
  Composer: (props: { onSubmit?: typeof mocks.composerSubmit }) => {
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
const turnRest = vi.fn<NonNullable<ChatBackend['turnRest']>>()
const stream = vi.fn<ChatBackend['stream']>(async function* () {})
const onAppendMessage = vi.fn()

async function open(
  conversation: Conversation,
  props: Partial<ComponentProps<typeof ChatView>> = {},
) {
  const noop = vi.fn()
  await act(async () => {
    root.render(
      <TooltipProvider>
        <ChatView
          key={conversation.id}
          conversation={conversation}
          backend={{ id: 'real', stream, turnRest } as unknown as ChatBackend}
          modelOptions={[{ id: 'anthropic::claude', label: 'Claude' }]}
          catalogLoading={false}
          onUpdateModel={noop}
          onUpdateThinkingLevel={noop}
          onUpdateWorkingDir={noop}
          onAppendMessage={onAppendMessage}
          onPatchMessage={noop}
          onCompactConversation={noop}
          {...props}
        />
      </TooltipProvider>,
    )
  })
  // Let the activation promise settle.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
}

const continueButton = () =>
  [...host.querySelectorAll('button')].find(
    (button) => button.textContent === 'Continue',
  )

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  mocks.activateWorkingDir.mockReset().mockResolvedValue(valid)
  mocks.setDraftText.mockReset()
  mocks.setDraftAttachments.mockReset()
  mocks.draftText = undefined
  mocks.ensureSession = undefined
  turnRest.mockReset().mockResolvedValue('ended')
  stream.mockClear()
  onAppendMessage.mockReset()
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.unstubAllGlobals()
})

it('offers Continue on an ended turn and sends it once', async () => {
  await open(done)
  expect(turnRest).toHaveBeenCalledWith('root')
  expect(host.textContent).toContain('Turn ended.')
  expect(continueButton()?.parentElement?.nextElementSibling).toBe(
    host.querySelector('textarea'),
  )

  // The send waits for the folder check; the row is already gone, so a
  // second click reaches neither a second send nor the draft.
  let folderChecked!: (result: typeof valid) => void
  mocks.activateWorkingDir.mockReturnValue(
    new Promise((resolve) => {
      folderChecked = resolve
    }),
  )
  const clicked = continueButton()
  await act(async () => clicked?.click())
  expect(continueButton()).toBeUndefined()
  expect(host.textContent).not.toContain('Turn ended.')
  expect(stream).not.toHaveBeenCalled()
  await act(async () => clicked?.click())
  await act(async () => folderChecked(valid))

  expect(stream).toHaveBeenCalledTimes(1)
  expect(stream.mock.calls[0]?.[0]).toBe('Continue')
  expect(onAppendMessage).toHaveBeenCalledWith(
    'root',
    expect.objectContaining({ role: 'user', content: 'Continue' }),
  )
  expect(mocks.setDraftText).not.toHaveBeenCalled()
})

it('leaves Continue in the draft while the chat cannot send', async () => {
  mocks.activateWorkingDir.mockReturnValue(new Promise(() => {}))
  await open(done)
  await act(async () => continueButton()?.click())
  expect(stream).not.toHaveBeenCalled()
  expect(mocks.setDraftText).toHaveBeenCalledWith('root', 'Continue')
})

it('keeps the draft when Continue fails before it is sent', async () => {
  mocks.draftText = 'Half a thought'
  mocks.ensureSession = () => Promise.reject(new Error('down'))
  await open(done)
  await act(async () => continueButton()?.click())
  expect(stream).not.toHaveBeenCalled()
  expect(mocks.setDraftText).toHaveBeenCalledExactlyOnceWith(
    'root',
    'Continue\n\nHalf a thought',
  )
  expect(mocks.setDraftAttachments).not.toHaveBeenCalled()
})

it.each([
  ['gives the caret to the composer', true, 1],
  ['raises no keyboard on a touch screen', false, 0],
])('%s when the clicked button goes', async (_, finePointer, times) => {
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: finePointer && query === DESKTOP_POINTER_QUERY,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }))
  const focused = vi.fn()
  const off = onComposerFocusRequest(focused)
  await open(done)
  expect(focused).not.toHaveBeenCalled()
  await act(async () => continueButton()?.click())
  expect(stream).toHaveBeenCalledTimes(1)
  expect(focused).toHaveBeenCalledTimes(times)
  off()
})

it('reads again when the transcript is back after a reconnect', async () => {
  turnRest.mockResolvedValueOnce('waiting')
  await open(done)
  expect(host.textContent).toContain('Waiting for a notification.')
  // The turn came and went while this tab was away: `done` before and after.
  await open({ ...done, hydrated: false })
  expect(host.textContent).not.toContain('Waiting for a notification.')
  expect(turnRest).toHaveBeenCalledTimes(1)
  await open(done)
  expect(turnRest).toHaveBeenCalledTimes(2)
  expect(continueButton()).toBeDefined()
})

it('reads nothing for a session that works, failed or never ran', async () => {
  await open({ ...done, id: 'working', status: 'working' })
  await open({ ...done, id: 'failed', status: 'error' })
  await open({ ...done, id: 'draft', status: undefined, draft: true })
  expect(turnRest).not.toHaveBeenCalled()
  expect(continueButton()).toBeUndefined()
})

it('hides while a reply streams here, before the session reads working', async () => {
  await open(done)
  expect(continueButton()).toBeDefined()
  stream.mockImplementationOnce(async function* () {
    yield { kind: 'assistant-token', token: 'On it' }
    await new Promise(() => {})
  })
  await act(async () => {
    void mocks.composerSubmit?.({ text: 'Go on', attachments: [] })
  })
  expect(stream).toHaveBeenCalledTimes(1)
  expect(continueButton()).toBeUndefined()
})

it('shows no row for a sub-agent, a side chat or a read-only session', async () => {
  await open({ ...done, id: 'child', parentId: 'root' })
  await open({ ...done, id: 'side' }, { side: { since: 1, empty: null } })
  await open({ ...done, id: 'imported', sessionMetadata: { read_only: true } })
  expect(turnRest).not.toHaveBeenCalled()
  expect(continueButton()).toBeUndefined()
})

it('shows no row on a backend that cannot tell', async () => {
  await open(done, {
    backend: { id: 'real', stream } as unknown as ChatBackend,
  })
  expect(host.querySelector('textarea')).not.toBeNull()
  expect(continueButton()).toBeUndefined()
})
