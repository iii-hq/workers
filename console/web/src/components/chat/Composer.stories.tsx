import type { Meta, StoryObj } from '@storybook/react-vite'
import { $createParagraphNode, $createTextNode, $getRoot } from 'lexical'
import { useState } from 'react'
import { fn } from 'storybook/test'
import type { FileHit } from '@/lib/file-search'
import { STATIC_FUNCTIONS } from '@/lib/functions'
import type {
  Attachment,
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
import { $createFileMentionNode } from './lexical/FileMentionNode'
import { $createFunctionMentionNode } from './lexical/FunctionMentionNode'
import { $createSlashCommandNode } from './lexical/SlashCommandNode'

/** Stand-in for the shell worker's quick-open search: a fixed tree,
    subsequence-filtered, so the `@` and `#` menus have files to show. */
const STORY_FILES: FileHit[] = [
  { path: 'README.md', kind: 'file' },
  { path: 'package.json', kind: 'file' },
  { path: 'src/', kind: 'dir' },
  { path: 'src/App.tsx', kind: 'file' },
  { path: 'src/main.tsx', kind: 'file' },
  { path: 'src/components/', kind: 'dir' },
  { path: 'src/components/chat/Composer.tsx', kind: 'file' },
  { path: 'src/components/chat/LexicalShell.tsx', kind: 'file' },
  { path: 'src/components/chat/lexical/MentionsPlugin.tsx', kind: 'file' },
  { path: 'src/components/chat/lexical/FileMentionNode.tsx', kind: 'file' },
  { path: 'src/lib/file-search.ts', kind: 'file' },
  { path: 'src/lib/mention-search.ts', kind: 'file' },
  { path: 'src/lib/functions.ts', kind: 'file' },
  { path: 'harness/src/turn_loop.rs', kind: 'file' },
  { path: 'harness/src/functions/turn.rs', kind: 'file' },
  { path: 'shell/src/code/functions/search.rs', kind: 'file' },
  { path: 'shell/ui/src/page/EditorPane.tsx', kind: 'file' },
  {
    path: 'docs/a very long file name that keeps going and going to test the ellipsis.md',
    kind: 'file',
  },
]

function isSubsequence(needle: string, haystack: string): boolean {
  let i = 0
  for (const ch of haystack) {
    if (ch === needle[i]) i++
    if (i === needle.length) return true
  }
  return needle.length === 0
}

async function storySearchFiles(query: string): Promise<FileHit[]> {
  const q = query.trim().toLowerCase()
  return STORY_FILES.filter((hit) => isSubsequence(q, hit.path.toLowerCase()))
}

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

/** Seed the editor with a file mention that carries a line window. */
function seedWithFileMention() {
  const root = $getRoot()
  root.clear()
  const para = $createParagraphNode()
  para.append($createTextNode('explain '))
  para.append(
    $createFileMentionNode('src/components/chat/Composer.tsx', {
      from: 12,
      to: 40,
    }),
  )
  para.append($createTextNode(' and '))
  para.append($createFileMentionNode('src/lib/functions.ts'))
  root.append(para)
}

/** Seed the editor with a skill invocation leading the sentence and a file mention in it. */
function seedWithSlashCommand() {
  const root = $getRoot()
  root.clear()
  const para = $createParagraphNode()
  para.append($createSlashCommandNode('/skill:coder/index'))
  para.append($createTextNode(' tighten the retry loop in '))
  para.append($createFileMentionNode('src/lib/dispatcher.ts'))
  para.append($createTextNode(', then run '))
  para.append($createSlashCommandNode('/skill:review-pr'))
  para.append($createTextNode(' on it'))
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
 * Stateful wrapper so the in-composer model + directory pickers actually
 * move, matching how the composer behaves inside the live chat surface.
 */
function ComposerHarness({
  initialModel = STORY_MODEL_OPTIONS[0].id,
  initialThinkingLevel = 'default',
  initialWorkingDir,
  initialContent,
  initialAttachments,
  isStreaming,
  openFileMention,
}: {
  initialModel?: ModelId
  initialThinkingLevel?: ThinkingLevel
  initialWorkingDir?: string
  initialContent?: () => void
  initialAttachments?: Attachment[]
  isStreaming?: boolean
  /** Make file pills clickable (logged to the Actions panel). */
  openFileMention?: boolean
}) {
  const [model, setModel] = useState<ModelId>(initialModel)
  const [thinkingLevel, setThinkingLevel] =
    useState<ThinkingLevel>(initialThinkingLevel)
  const [workingDir, setWorkingDir] = useState(initialWorkingDir)
  return (
    <Composer
      model={model}
      modelOptions={STORY_MODEL_OPTIONS}
      functionEntries={STATIC_FUNCTIONS}
      searchFiles={storySearchFiles}
      onOpenFileMention={openFileMention ? fn() : undefined}
      permissionMode="manual"
      thinkingLevel={thinkingLevel}
      showWorkingDir={initialWorkingDir !== undefined}
      workingDir={workingDir}
      onThinkingLevelChange={setThinkingLevel}
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
      initialModel="anthropic::claude-opus-4-7"
      initialContent={seedWithText}
    />
  ),
}

export const FunctionMention: Story = {
  name: 'pre-seeded with a function mention',
  render: () => (
    <ComposerHarness
      initialModel="openai::gpt-5"
      initialContent={seedWithMention}
    />
  ),
}

export const FileMention: Story = {
  name: 'pre-seeded with file mentions (one with lines)',
  render: () => (
    <ComposerHarness
      initialModel="openai::gpt-5"
      initialContent={seedWithFileMention}
      openFileMention
    />
  ),
}

export const SlashCommands: Story = {
  name: 'pre-seeded with skill invocations (command pills)',
  render: () => (
    <ComposerHarness
      initialModel="openai::gpt-5"
      initialContent={seedWithSlashCommand}
      openFileMention
    />
  ),
}

export const WithAttachments: Story = {
  name: 'with attachments',
  render: () => (
    <ComposerHarness
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

export const MobileProjectStrip: Story = {
  name: 'mobile, with project strip',
  render: () => (
    <div className="max-w-[390px]">
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
    <ComposerHarness initialModel="openai::gpt-5-mini" isStreaming />
  ),
}
