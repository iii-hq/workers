// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  type ConversationsApi,
  useConversations,
} from '@/hooks/use-conversations'
import type { AgentEntry } from '@/lib/backend/directory-prompts'
import { getIiiClient } from '@/lib/iii-client'
import type { ExamplePrompt } from '@/lib/onboarding/prompts'
import { fetchTranscriptTail, listSessions } from '@/lib/sessions/api'
import type { ModelOption } from '@/types/chat'
import { openExamplePrompt } from './example-prompt'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
vi.mock('@/lib/sessions/api', async (original) => ({
  ...(await original<typeof import('@/lib/sessions/api')>()),
  listSessions: vi.fn(),
  getSession: vi.fn(),
  fetchTranscriptTail: vi.fn(),
  ensureSession: vi.fn(),
}))
vi.mock('@/lib/sessions/events', () => ({
  subscribeSessionDirectory: vi.fn(() => () => {}),
  subscribeSessionTranscript: () => () => {},
}))

const MODELS: ModelOption[] = [
  {
    id: 'anthropic::claude-sonnet-5-5',
    label: 'claude sonnet 5.5',
    supportsThinking: true,
    reasoningEfforts: [
      { effort: 'low' },
      { effort: 'medium' },
      { effort: 'high' },
    ],
  },
  { id: 'openai::gpt-6.1-sol', label: 'gpt 6.1 sol', supportsThinking: false },
]

const BUILDER: AgentEntry = {
  id: 'ade-worker-builder',
  name: 'Create an app or tool',
  description: 'Builds a worker',
  logo: null,
  icon: 'code',
  model: null,
  skill_count: 3,
  modified_at: '2026-10-01T00:00:00Z',
}

const TODO: ExamplePrompt = {
  title: 'Build a TODO app',
  agent: 'ade-worker-builder',
  prompt: 'Build a TODO app. Show how many todos are still open.',
  models: [
    { provider: 'claude-code', model: 'claude-sonnet-5-5', effort: 'medium' },
    { provider: 'anthropic', model: 'claude-sonnet-5-5', effort: 'medium' },
    { provider: 'openai', model: 'gpt-6.1-sol', effort: 'medium' },
  ],
}

let api: ConversationsApi
let root: Root | null = null

function Probe() {
  api = useConversations(
    MODELS.map((model) => model.id),
    true,
    true,
  )
  return null
}

async function boot() {
  const host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () => root?.render(<Probe />))
}

/** Open `prompt` the way the wizard does; returns the chat and what was shown. */
async function open(
  prompt: ExamplePrompt,
  agents: AgentEntry[] = [BUILDER],
  models: ModelOption[] = MODELS,
) {
  const shown: string[] = []
  let id = ''
  await act(async () => {
    id = openExamplePrompt(
      {
        ...api,
        openConversation: (chat) => {
          shown.push(chat)
          api.select(chat)
        },
        modelOptions: models,
      },
      prompt,
      agents,
    )
  })
  const chat = api.conversations.find((conversation) => conversation.id === id)
  if (!chat) throw new Error('no chat opened')
  return { chat, shown }
}

beforeEach(async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.clearAllMocks()
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  localStorage.clear()
  vi.mocked(getIiiClient).mockResolvedValue({
    addConnectionStateListener: (listener: (state: string) => void) => {
      listener('connected')
      return () => {}
    },
  } as never)
  vi.mocked(listSessions).mockResolvedValue([])
  vi.mocked(fetchTranscriptTail).mockResolvedValue({
    items: [],
    hasMore: false,
  })
  await boot()
})

afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  root = null
})

describe('openExamplePrompt', () => {
  it('opens a new chat with the prompt waiting to be sent, its profile and the first model this machine has', async () => {
    const { chat, shown } = await open(TODO)
    expect(chat).toMatchObject({
      draft: true,
      messages: [],
      model: 'anthropic::claude-sonnet-5-5',
      thinkingLevel: 'medium',
      agentProfile: { id: 'ade-worker-builder', name: 'Create an app or tool' },
    })
    // In the message box, not sent.
    expect(api.getDraftText(chat.id)).toBe(TODO.prompt)
    expect(api.activeId).toBe(chat.id)
    expect(shown).toEqual([chat.id])
  })

  it('keeps the chat’s usual model when no listed model is available', async () => {
    const { chat: usual } = await open({ ...TODO, models: [] })
    const { chat } = await open({
      ...TODO,
      models: [
        { provider: 'deepseek', model: 'deepseek-flash', effort: 'high' },
      ],
    })
    expect(chat.model).toBe(usual.model)
    expect(chat.thinkingLevel).toBe(usual.thinkingLevel)
  })

  it('leaves out an effort the chosen model does not offer', async () => {
    const { chat } = await open({
      ...TODO,
      models: [{ provider: 'openai', model: 'gpt-6.1-sol', effort: 'high' }],
    })
    expect(chat.model).toBe('openai::gpt-6.1-sol')
    expect(chat.thinkingLevel).not.toBe('high')
  })

  it('selects no profile the Directory does not serve, and still prefills the text', async () => {
    const { chat } = await open(TODO, [])
    expect(chat.agentProfile).toBeUndefined()
    expect(api.getDraftText(chat.id)).toBe(TODO.prompt)
  })
})
