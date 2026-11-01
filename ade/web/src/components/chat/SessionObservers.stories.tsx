import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { fn } from 'storybook/test'
import type { SessionObserverInfo } from '@/lib/backend/observers'
import type { ModelId, ModelOption } from '@/types/chat'
import { Composer } from './Composer'
import { SessionObservers } from './SessionObservers'

const OWNER = 'code-review-console-7a7a991e-2394-4636-ac45-59964ebc57bc-k4m8'
const REGISTERED_AT = Date.UTC(2026, 8, 3, 9, 30)

/** A live `state::on-changed` wake into this chat, as the review orchestration observes them. */
function stateWake(
  id: string,
  label: string,
  key: string,
  overrides: Partial<SessionObserverInfo> = {},
): SessionObserverInfo {
  return {
    id: `obs_${id}`,
    functionId: 'state::on-changed',
    delivery: { kind: 'notify' },
    filters: {
      'input.scope': { $eq: `${OWNER}/review` },
      'input.key': { $eq: key },
    },
    label,
    once: true,
    fires: 0,
    createdAt: REGISTERED_AT,
    ...overrides,
  }
}

/** The same wake after it fired: a retired ghost the transcript still knows. */
function consumed(id: string, label: string, key: string): SessionObserverInfo {
  return stateWake(id, label, key, {
    fired: true,
    fires: 1,
    outcome: 'delivered',
    retirementReason: 'once_consumed',
    firedAt: REGISTERED_AT + 4 * 60_000,
  })
}

const reviewAgents: SessionObserverInfo[] = [
  stateWake('impl3', 'Agent 3 implementation', 'agent-3/implementation'),
  stateWake('research2', 'Agent 2 research', 'agent-2/research'),
  stateWake('findings1', 'Agent 1 findings', 'agent-1/findings'),
  consumed('ready1', 'Agent 1 ready', 'agent-1/ready'),
  consumed('ready2', 'Agent 2 ready', 'agent-2/ready'),
  consumed('ready3', 'Agent 3 ready', 'agent-3/ready'),
]

/** Functions beyond `state::on-changed`, including one that does not exist yet. */
const mixedSources: SessionObserverInfo[] = [
  {
    id: 'obs_nightly',
    functionId: 'cron::on-tick',
    delivery: { kind: 'call', functionId: 'reports::nightly' },
    args: { expression: '0 0 2 * * * *' },
    label: 'nightly report',
    fires: 12,
    maxFires: 30,
    createdAt: REGISTERED_AT,
  },
  {
    id: 'obs_deadline',
    functionId: 'harness::on-timer',
    delivery: { kind: 'notify' },
    args: { at: REGISTERED_AT + 45 * 60_000 },
    once: true,
    expiresAt: REGISTERED_AT + 45 * 60_000,
    createdAt: REGISTERED_AT,
  },
  {
    id: 'obs_sensors',
    functionId: 'mqtt::on-message',
    delivery: { kind: 'notify' },
    args: { topic: 'sensors/#', qos: 1, retained: true },
    label: 'sensor stream',
    conditions: [{ path: 'payload.temp', op: 'gt', value: 40 }],
    createdAt: REGISTERED_AT,
  },
  stateWake('budget', 'budget exhausted', 'spend/limit', {
    fired: true,
    retirementReason: 'max_fires',
  }),
]

/** Resolve the state probe from the key name so stories show both notes. */
const checkStateKey = async (_scope: string | undefined, key: string) =>
  key.endsWith('ready') || key.endsWith('findings')

const STORY_MODEL: ModelOption = {
  id: 'openai::gpt-5',
  label: 'gpt-5',
  contextWindow: 400_000,
  supportsThinking: true,
}

/** The chat footer as ChatView stacks it: observers strip, then the composer. */
function Footer({ observers }: { observers: SessionObserverInfo[] }) {
  const [model, setModel] = useState<ModelId>(STORY_MODEL.id)
  const [workingDir, setWorkingDir] = useState('/workspace/video-making')
  return (
    <div className="mx-auto max-w-[760px]">
      <SessionObservers
        observers={observers}
        onDispose={fn()}
        onClearAll={fn()}
        checkStateKey={checkStateKey}
      />
      <Composer
        model={model}
        modelOptions={[STORY_MODEL]}
        permissionMode="manual"
        thinkingLevel="default"
        showWorkingDir
        workingDir={workingDir}
        onWorkingDirChange={setWorkingDir}
        onModelChange={setModel}
        onThinkingLevelChange={fn()}
        onPermissionModeChange={fn()}
        onSubmit={fn()}
      />
    </div>
  )
}

const meta = {
  title: 'Chat/SessionObservers',
  component: SessionObservers,
  parameters: { layout: 'padded' },
  args: {
    observers: reviewAgents,
    onDispose: fn(),
    onClearAll: fn(),
    checkStateKey,
  },
} satisfies Meta<typeof SessionObservers>

export default meta
type Story = StoryObj<typeof meta>

export const Collapsed: Story = {
  name: 'collapsed (default)',
}

export const Expanded: Story = {
  name: 'expanded, live + inactive rows',
  args: { defaultExpanded: true },
}

export const MixedSources: Story = {
  name: 'expanded, mixed sources',
  args: { observers: mixedSources, defaultExpanded: true },
}

export const NoClearAll: Story = {
  name: 'without clear all (read-only backend)',
  args: { onClearAll: undefined, defaultExpanded: true },
}

export const AboveComposer: Story = {
  name: 'stacked above the composer',
  render: (args) => <Footer observers={args.observers} />,
}

export const Phone: Story = {
  name: 'phone width, expanded',
  args: { defaultExpanded: true },
  render: (args) => (
    <div className="max-w-[390px]">
      <SessionObservers {...args} />
    </div>
  ),
}
