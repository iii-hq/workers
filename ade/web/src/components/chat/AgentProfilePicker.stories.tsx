import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import type { AgentEntry } from '@/lib/backend/directory-prompts'
import type { AgentProfileSnapshot } from '@/types/chat'
import { AgentProfilePicker } from './AgentProfilePicker'
import type { SystemPromptState } from './system-prompt-selection'

const AGENTS: AgentEntry[] = [
  {
    id: 'ade-worker-builder',
    name: 'ADE Worker Builder',
    description:
      'Plans an ADE worker with the user — an iii worker whose functions, observers and configuration are designed before any code is written.',
    logo: null,
    icon: 'code',
    model: null,
    skill_count: 3,
    source: { kind: 'kit', kit: 'ade-kit' },
    modified_at: '2026-11-01T00:00:00Z',
  },
  {
    id: 'agent-profile-creator',
    name: 'Agent Profile Creator',
    description:
      'Composes a new agent profile with the user — interviews until the role, its boundaries, its skills and its model are settled.',
    logo: null,
    icon: 'docs',
    model: null,
    skill_count: 1,
    modified_at: '2026-11-01T00:00:00Z',
  },
  {
    id: 'default',
    name: 'Default',
    description:
      'Use for general questions and development tasks in your project.',
    logo: null,
    icon: 'agent',
    model: null,
    skill_count: 0,
    modified_at: '2026-11-01T00:00:00Z',
  },
]

const PROMPT: SystemPromptState = {
  choice: 'default',
  strategy: 'enrich',
  namedBody: '',
  customText: '',
  addons: [],
}

function Harness({
  locked = false,
  withUpdate = false,
}: {
  locked?: boolean
  withUpdate?: boolean
}) {
  const [profile, setProfile] = useState<AgentProfileSnapshot | undefined>({
    id: 'default',
    name: 'Default',
    icon: 'agent',
  })
  return (
    <div className="flex h-[36rem] items-end p-6">
      <AgentProfilePicker
        systemPrompt={PROMPT}
        onSystemPromptChange={() => {}}
        agentEntries={AGENTS}
        agentProfile={profile}
        onAgentProfileChange={setProfile}
        selectionLocked={locked}
        kitUpdates={
          withUpdate
            ? [
                {
                  kit: 'ade-kit',
                  current: '1.2.0',
                  available: '2.0.0',
                  major: true,
                },
              ]
            : []
        }
        details={
          <span className="text-sm text-ink-faint">Model · Sonnet 5</span>
        }
      />
    </div>
  )
}

const meta = {
  title: 'Chat/AgentProfilePicker',
  component: Harness,
  parameters: { layout: 'fullscreen' },
} satisfies Meta<typeof Harness>

export default meta
type Story = StoryObj<typeof meta>

export const Open: Story = { args: {} }
export const Locked: Story = { args: { locked: true } }
export const WithUpdate: Story = { args: { withUpdate: true } }
