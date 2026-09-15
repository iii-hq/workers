import { renderToStaticMarkup } from 'react-dom/server'
import { beforeEach, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ChatBackend } from '@/lib/backend'
import { buildTurnMetadata } from '@/lib/backend/real'
import type { Conversation } from '@/types/chat'
import { ChatView } from './ChatView'
import type { ComposerSubmitPayload } from './Composer'
import { DEFAULT_SYSTEM_PROMPT_STATE } from './system-prompt-selection'

const statusTrigger = vi.hoisted(() => vi.fn())
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ trigger: statusTrigger }),
}))
const composer = vi.hoisted(() => ({
  props: null as null | {
    model: string | null
    submitBlocked: boolean
    onSubmit: (payload: ComposerSubmitPayload) => Promise<void>
  },
}))
vi.mock('./Composer', () => ({
  Composer: (props: NonNullable<typeof composer.props>) => {
    composer.props = props
    return <textarea />
  },
}))
const stream = vi.fn(async function* () {})
const session: Conversation = {
  id: 'new-native-copy',
  title: 'Imported',
  model: null,
  hydrated: true,
  started: true,
  messages: [
    {
      id: 'original-assistant',
      role: 'assistant',
      content: 'Original answer',
      model: 'external-model',
      createdAt: 1,
    },
  ],
  createdAt: 1,
  updatedAt: 2,
  sessionMetadata: {
    external_source: 'codex',
    external_session_id: 'source',
    source_cwd: '/source/project',
  },
}
function render(conversation: Conversation) {
  const noop = vi.fn()
  renderToStaticMarkup(
    <TooltipProvider>
      <ChatView
        conversation={conversation}
        backend={{ id: 'mock', stream } as unknown as ChatBackend}
        modelOptions={[{ id: 'ade::chosen', label: 'Chosen ADE model' }]}
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
  if (!composer.props) throw new Error('Native composer did not mount')
  return composer.props
}
beforeEach(() => {
  statusTrigger.mockReset().mockResolvedValue(null)
  stream.mockClear()
  composer.props = null
})
it('does not adopt the source model or project and blocks ordinary sends and compaction until configured', async () => {
  for (const conversation of [
    session,
    { ...session, model: 'ade::chosen' },
    { ...session, workingDir: '/ade/project' },
  ]) {
    const props = render(conversation)
    expect(props.submitBlocked).toBe(true)
    if (!conversation.model) expect(props.model).toBeNull()
    await props.onSubmit({ text: 'Continue', attachments: [] })
    await props.onSubmit({ text: '/compact', attachments: [] })
  }
  expect(stream).not.toHaveBeenCalled()
})
it('continues through the normal backend with the chosen ADE model and project', async () => {
  const props = render({
    ...session,
    model: 'ade::chosen',
    workingDir: '/ade/project',
  })
  expect(props.submitBlocked).toBe(false)
  await props.onSubmit({ text: 'Continue', attachments: [] })
  expect(stream).toHaveBeenCalledTimes(1)
  expect(stream.mock.calls[0]).toEqual([
    'Continue',
    'ade::chosen',
    expect.objectContaining({
      sessionId: 'new-native-copy',
      workingDir: '/ade/project',
    }),
  ])
  expect(
    buildTurnMetadata('new-native-copy', 'message', '/ade/project'),
  ).toMatchObject({ fs_scope: { root: '/ade/project' } })
})

it('sends selected prompt and skills on the first native turn despite imported assistant text', async () => {
  const props = render({
    ...session,
    model: 'ade::chosen',
    workingDir: '/ade/project',
    systemPrompt: {
      ...DEFAULT_SYSTEM_PROMPT_STATE,
      choice: 'custom',
      customText: 'ADE instructions',
      strategy: 'override',
    },
    skills: ['ade-skill'],
  })
  await props.onSubmit({ text: 'Continue', attachments: [] })
  expect(statusTrigger).toHaveBeenCalledWith('harness::status', {
    session_id: 'new-native-copy',
    verbose: true,
  })
  expect(stream).toHaveBeenCalledWith(
    'Continue',
    'ade::chosen',
    expect.objectContaining({
      systemPrompt: { body: 'ADE instructions', strategy: 'override' },
      skills: ['ade-skill'],
    }),
  )
})
it('sends an explicitly selected agent only when no native turn exists', async () => {
  const configured = {
    ...session,
    model: 'ade::chosen',
    workingDir: '/ade/project',
    agentProfile: { id: 'ade-agent', name: 'ADE agent' },
  }
  await render(configured).onSubmit({ text: 'Continue', attachments: [] })
  expect(stream).toHaveBeenCalledWith(
    'Continue',
    'ade::chosen',
    expect.objectContaining({
      agent: 'ade-agent',
      systemPrompt: null,
      skills: undefined,
    }),
  )
  stream.mockClear()
  statusTrigger.mockResolvedValue({ turn_id: 'native-turn' })
  await render(configured).onSubmit({ text: 'Continue', attachments: [] })
  expect(stream).toHaveBeenCalledWith(
    'Continue',
    'ade::chosen',
    expect.not.objectContaining({ agent: 'ade-agent' }),
  )
})
it('inherits established native turn configuration after reopening without requiring a new project selection', async () => {
  statusTrigger.mockResolvedValue({ turn_id: 'native-turn' })
  const props = render({
    ...session,
    model: 'ade::chosen',
    workingDir: null,
    systemPrompt: {
      ...DEFAULT_SYSTEM_PROMPT_STATE,
      choice: 'custom',
      customText: 'Old selection',
    },
    skills: ['old-skill'],
  })
  await props.onSubmit({ text: 'Continue', attachments: [] })
  expect(stream).toHaveBeenCalledWith(
    'Continue',
    'ade::chosen',
    expect.objectContaining({
      workingDir: null,
      systemPrompt: null,
      skills: undefined,
    }),
  )
})

it('checks turn state again on every send when a previous native turn disappears', async () => {
  const props = render({ ...session, model: 'ade::chosen', workingDir: null })
  statusTrigger.mockResolvedValueOnce({ turn_id: 'native-turn' })
  await props.onSubmit({ text: 'Continue', attachments: [] })
  expect(stream).toHaveBeenCalledTimes(1)
  await props.onSubmit({ text: 'Continue after reset', attachments: [] })
  expect(statusTrigger).toHaveBeenCalledTimes(2)
  expect(stream).toHaveBeenCalledTimes(1)
})
it('allows only one first submission while its native turn lookup is pending', async () => {
  let resolveStatus!: (status: null) => void
  statusTrigger.mockReturnValueOnce(
    new Promise((resolve) => {
      resolveStatus = resolve
    }),
  )
  const props = render({
    ...session,
    model: 'ade::chosen',
    workingDir: '/ade/project',
  })
  const first = props.onSubmit({ text: 'First', attachments: [] })
  const second = props.onSubmit({ text: 'Duplicate', attachments: [] })
  await second
  expect(statusTrigger).toHaveBeenCalledTimes(1)
  expect(stream).not.toHaveBeenCalled()
  resolveStatus(null)
  await first
  expect(stream).toHaveBeenCalledTimes(1)
  expect(stream).toHaveBeenCalledWith('First', 'ade::chosen', expect.anything())
  await props.onSubmit({ text: 'Next', attachments: [] })
  expect(stream).toHaveBeenCalledTimes(2)
})
