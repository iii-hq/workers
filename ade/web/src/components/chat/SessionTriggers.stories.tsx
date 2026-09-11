import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { fn } from 'storybook/test'
import type { SessionTriggerInfo } from '@/lib/backend/triggers'
import type { ModelId, ModelOption } from '@/types/chat'
import { Composer } from './Composer'
import { SessionTriggers } from './SessionTriggers'

const OWNER = 'code-review-console-7a7a991e-2394-4636-ac45-59964ebc57bc-k4m8'
const REGISTERED_AT = Date.UTC(2026, 8, 3, 9, 30)

/** A live `state` wake into this chat, as the review orchestration registers them. */
function stateWake(
  id: string,
  label: string,
  key: string,
  overrides: Partial<SessionTriggerInfo> = {},
): SessionTriggerInfo {
  return {
    id: `sub_${id}`,
    triggerId: `trg_${id}`,
    triggerType: 'state',
    delivery: { kind: 'notify' },
    config: { scope: `${OWNER}/review`, key },
    label,
    once: true,
    fires: 0,
    createdAt: REGISTERED_AT,
    ...overrides,
  }
}

/** The same wake after it fired: a retired ghost the transcript still knows. */
function consumed(id: string, label: string, key: string): SessionTriggerInfo {
  return stateWake(id, label, key, {
    fired: true,
    fires: 1,
    outcome: 'delivered',
    retirementReason: 'once_consumed',
    firedAt: REGISTERED_AT + 4 * 60_000,
  })
}

const reviewAgents: SessionTriggerInfo[] = [
  stateWake('impl3', 'Agent 3 implementation', 'agent-3/implementation'),
  stateWake('research2', 'Agent 2 research', 'agent-2/research'),
  stateWake('findings1', 'Agent 1 findings', 'agent-1/findings'),
  consumed('ready1', 'Agent 1 ready', 'agent-1/ready'),
  consumed('ready2', 'Agent 2 ready', 'agent-2/ready'),
  consumed('ready3', 'Agent 3 ready', 'agent-3/ready'),
]

/** Sources beyond `state`, including one that does not exist yet. */
const mixedSources: SessionTriggerInfo[] = [
  {
    id: 'sub_nightly',
    triggerId: 'trg_nightly',
    triggerType: 'cron',
    delivery: { kind: 'call', functionId: 'reports::nightly' },
    config: { expr: '0 2 * * *', tz: 'America/Sao_Paulo' },
    label: 'nightly report',
    fires: 12,
    maxFires: 30,
    createdAt: REGISTERED_AT,
  },
  {
    id: 'sub_deadline',
    triggerId: 'trg_deadline',
    triggerType: 'timer',
    delivery: { kind: 'notify' },
    config: { after: '45m' },
    once: true,
    expiresAt: REGISTERED_AT + 45 * 60_000,
    createdAt: REGISTERED_AT,
  },
  {
    id: 'sub_sensors',
    triggerId: 'trg_sensors',
    triggerType: 'mqtt',
    delivery: { kind: 'notify' },
    config: { topic: 'sensors/#', qos: 1, retained: true },
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

/** The chat footer as ChatView stacks it: triggers strip, then the composer. */
function Footer({ triggers }: { triggers: SessionTriggerInfo[] }) {
  const [model, setModel] = useState<ModelId>(STORY_MODEL.id)
  const [workingDir, setWorkingDir] = useState('/workspace/video-making')
  return (
    <div className="mx-auto max-w-[760px]">
      <SessionTriggers
        triggers={triggers}
        onUnregister={fn()}
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
  title: 'Chat/SessionTriggers',
  component: SessionTriggers,
  parameters: { layout: 'padded' },
  args: {
    triggers: reviewAgents,
    onUnregister: fn(),
    onClearAll: fn(),
    checkStateKey,
  },
} satisfies Meta<typeof SessionTriggers>

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
  args: { triggers: mixedSources, defaultExpanded: true },
}

export const NoClearAll: Story = {
  name: 'without clear all (read-only backend)',
  args: { onClearAll: undefined, defaultExpanded: true },
}

export const AboveComposer: Story = {
  name: 'stacked above the composer',
  render: (args) => <Footer triggers={args.triggers} />,
}

export const Phone: Story = {
  name: 'phone width, expanded',
  args: { defaultExpanded: true },
  render: (args) => (
    <div className="max-w-[390px]">
      <SessionTriggers {...args} />
    </div>
  ),
}
