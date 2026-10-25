import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import type { AgentEntry } from '@/lib/backend/directory-prompts'
import type { AgentProfileSnapshot, ModelOption } from '@/types/chat'
import { AgentProfilePicker } from './AgentProfilePicker'
import { Composer } from './Composer'
import { EmptyState } from './EmptyState'
import { DEFAULT_SYSTEM_PROMPT_STATE } from './system-prompt-selection'

const agents: AgentEntry[] = [
  {
    id: 'default',
    name: 'Default',
    description: 'General questions and development tasks.',
    logo: null,
    icon: null,
    model: null,
    skill_count: null,
    modified_at: '',
  },
  {
    id: 'reviewer',
    name: 'Reviewer',
    description: 'Reviews changes and identifies issues before you ship.',
    logo: null,
    icon: 'review',
    model: null,
    skill_count: 2,
    modified_at: '',
  },
]
const models: ModelOption[] = [
  {
    id: 'anthropic::claude-opus-5',
    label: 'Claude Opus 5',
    contextWindow: 1_000_000,
  },
  { id: 'openai::gpt-5', label: 'GPT-5', contextWindow: 400_000 },
]

function NewConversation({
  catalog = agents,
}: {
  catalog?: AgentEntry[] | null
}) {
  const [profile, setProfile] = useState<AgentProfileSnapshot>()
  const [prompt, setPrompt] = useState(DEFAULT_SYSTEM_PROMPT_STATE)
  const [model, setModel] = useState(models[0].id)
  const [workingDir, setWorkingDir] = useState('/workspace/Movies')
  const [messages, setMessages] = useState<Array<{ id: string; text: string }>>(
    [],
  )
  const welcome = messages.length === 0
  return (
    <section className="conversation-surface @container flex h-dvh min-w-0 flex-col bg-bg text-ink">
      <div
        className="chat-conversation-body flex min-h-0 flex-1 flex-col"
        data-welcome={welcome || undefined}
      >
        {welcome ? (
          <EmptyState variant="ready" />
        ) : (
          <div className="min-h-0 flex-1 overflow-y-auto px-6 py-8">
            <div className="mx-auto max-w-[760px]">
              {messages.map((message) => (
                <p key={message.id} className="mb-4 rounded-xl bg-surface p-4">
                  {message.text}
                </p>
              ))}
            </div>
          </div>
        )}
        <footer className="px-4 pt-2 pb-6">
          <div className="mx-auto max-w-[760px]">
            <Composer
              model={model}
              modelOptions={models}
              onModelChange={setModel}
              thinkingLevel="default"
              onThinkingLevelChange={() => {}}
              permissionMode="manual"
              showPermissionMode={false}
              onPermissionModeChange={() => {}}
              workingDir={workingDir}
              showWorkingDir
              onWorkingDirChange={setWorkingDir}
              autoFocus={false}
              showSuggestions={welcome}
              idlePlaceholder={
                welcome ? 'Explain how this project works…' : 'Send a message…'
              }
              onSubmit={({ text }) =>
                setMessages((previous) => [
                  ...previous,
                  { id: crypto.randomUUID(), text },
                ])
              }
              agentPicker={
                <AgentProfilePicker
                  systemPrompt={prompt}
                  onSystemPromptChange={setPrompt}
                  agentProfile={profile}
                  onAgentProfileChange={setProfile}
                  agentEntries={catalog}
                  preselectDefaultAgent
                  selectionLocked={!welcome}
                  details={
                    <button
                      type="button"
                      className="rounded-lg bg-surface px-3 py-2"
                    >
                      Runtime settings
                    </button>
                  }
                />
              }
            />
          </div>
        </footer>
      </div>
    </section>
  )
}

const meta = {
  title: 'Chat/NewConversation',
  component: NewConversation,
  parameters: { layout: 'fullscreen' },
} satisfies Meta<typeof NewConversation>
export default meta
type Story = StoryObj<typeof meta>
export const Ready: Story = {}
export const LoadingProfiles: Story = { args: { catalog: null } }
export const NoProfiles: Story = { args: { catalog: [] } }
