import type { Meta, StoryObj } from '@storybook/react-vite'
import { $createParagraphNode, $createTextNode, $getRoot } from 'lexical'
import { useState } from 'react'
import { fn } from 'storybook/test'
import { STATIC_FUNCTIONS } from '@/lib/functions'
import type {
  Attachment,
  Mode,
  ModelId,
  ModelOption,
  ThinkingLevel,
} from '@/types/chat'

const STORY_MODEL_OPTIONS: ModelOption[] = [
  {
    id: 'openai::gpt-5',
    label: 'gpt-5',
    contextWindow: 400_000,
    supportsThinking: true,
    reasoningEfforts: [
      { effort: 'none', description: 'fastest responses without reasoning' },
      { effort: 'minimal', description: 'minimal reasoning overhead' },
      { effort: 'low', description: 'quick reasoning for routine tasks' },
      { effort: 'medium', description: 'balanced reasoning and latency' },
      { effort: 'high', description: 'deeper reasoning for complex tasks' },
      { effort: 'xhigh', description: 'extended reasoning for hard tasks' },
      { effort: 'ultra', description: 'maximum model-native reasoning' },
    ],
  },
  {
    id: 'anthropic::claude-opus-4-7',
    label: 'claude opus 4.7',
    contextWindow: 1_000_000,
    supportsThinking: true,
  },
  {
    id: 'openai::gpt-5-mini',
    label: 'gpt-5 mini',
    contextWindow: 400_000,
    supportsThinking: true,
  },
  {
    id: 'codex::gpt-5.6-terra',
    label: 'gpt-5.6-terra (codex)',
    contextWindow: 400_000,
    supportsThinking: true,
    reasoningEfforts: [
      { effort: 'low', description: 'quick reasoning for routine tasks' },
      { effort: 'medium', description: 'balanced reasoning and latency' },
      { effort: 'high', description: 'deeper reasoning for complex tasks' },
    ],
  },
  {
    id: 'openai::gpt-4.1',
    label: 'gpt-4.1',
    contextWindow: 1_000_000,
    supportsThinking: false,
  },
]

import { Composer } from './Composer'
import { $createFunctionMentionNode } from './lexical/FunctionMentionNode'

const sampleAttachments: Attachment[] = [
  { id: 'spec', name: 'spec.md', size: 4_312, type: 'text/markdown' },
  { id: 'arch', name: 'architecture.svg', size: 18_440, type: 'image/svg+xml' },
]

/** Seed the editor with a sentence containing a function mention. */
function seedWithMention() {
  const root = $getRoot()
  root.clear()
  const para = $createParagraphNode()
  para.append($createTextNode('ping '))
  para.append($createFunctionMentionNode('engine::echo'))
  para.append($createTextNode(' with my prompt'))
  root.append(para)
}

/** Seed the editor with plain text only. */
function seedWithText() {
  const root = $getRoot()
  root.clear()
  const para = $createParagraphNode()
  para.append($createTextNode('how do i wire the engine into the agent loop?'))
  root.append(para)
}

/**
 * Stateful wrapper so the in-composer mode + model pickers actually move,
 * matching how the composer behaves inside the live chat surface.
 */
function ComposerHarness({
  initialMode = 'agent',
  initialModel = STORY_MODEL_OPTIONS[0].id,
  initialThinkingLevel = 'default',
  initialWorkingDir,
  initialContent,
  initialAttachments,
  isStreaming,
}: {
  initialMode?: Mode
  initialModel?: ModelId
  initialThinkingLevel?: ThinkingLevel
  initialWorkingDir?: string
  initialContent?: () => void
  initialAttachments?: Attachment[]
  isStreaming?: boolean
}) {
  const [mode, setMode] = useState<Mode>(initialMode)
  const [model, setModel] = useState<ModelId>(initialModel)
  const [thinkingLevel, setThinkingLevel] =
    useState<ThinkingLevel>(initialThinkingLevel)
  const [workingDir, setWorkingDir] = useState(initialWorkingDir)
  return (
    <Composer
      mode={mode}
      model={model}
      modelOptions={STORY_MODEL_OPTIONS}
      functionEntries={STATIC_FUNCTIONS}
      permissionMode="manual"
      thinkingLevel={thinkingLevel}
      showWorkingDir={initialWorkingDir !== undefined}
      workingDir={workingDir}
      onThinkingLevelChange={setThinkingLevel}
      onModeChange={setMode}
      onModelChange={setModel}
      onWorkingDirChange={setWorkingDir}
      onPermissionModeChange={fn()}
      onSubmit={fn()}
      onStop={fn()}
      initialContent={initialContent}
      initialAttachments={initialAttachments}
      isStreaming={isStreaming}
    />
  )
}

// Every variant renders through `ComposerHarness` (the composer has many
// required, conversation-shaped props), so the meta stays loose rather than
// declaring `component: Composer` and forcing per-story `args`.
const meta = {
  title: 'Chat/Composer',
  parameters: { layout: 'padded' },
} satisfies Meta

export default meta
type Story = StoryObj

export const Default: Story = {
  name: 'default (empty)',
  render: () => <ComposerHarness />,
}

export const PreSeededText: Story = {
  name: 'pre-seeded with plain text',
  render: () => (
    <ComposerHarness
      initialMode="ask"
      initialModel="anthropic::claude-opus-4-7"
      initialContent={seedWithText}
    />
  ),
}

export const FunctionMention: Story = {
  name: 'pre-seeded with a function mention',
  render: () => (
    <ComposerHarness
      initialMode="agent"
      initialModel="openai::gpt-5"
      initialContent={seedWithMention}
    />
  ),
}

export const WithAttachments: Story = {
  name: 'with attachments',
  render: () => (
    <ComposerHarness
      initialMode="agent"
      initialModel="openai::gpt-5"
      initialAttachments={sampleAttachments}
    />
  ),
}

export const WithReasoningEffort: Story = {
  name: 'selected reasoning effort',
  render: () => (
    <div className="max-w-[640px]">
      <ComposerHarness
        initialModel="codex::gpt-5.6-terra"
        initialThinkingLevel="medium"
        initialWorkingDir="/workspace/workers"
      />
    </div>
  ),
}

export const Streaming: Story = {
  name: 'disabled (streaming)',
  render: () => (
    <ComposerHarness
      initialMode="plan"
      initialModel="openai::gpt-5-mini"
      isStreaming
    />
  ),
}
