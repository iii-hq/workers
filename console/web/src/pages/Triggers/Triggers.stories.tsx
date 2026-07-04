import type { Meta, StoryObj } from '@storybook/react-vite'
import type { RegisteredTriggerSummary } from '@/components/chat/engine/parsers'
import { TriggersList } from './index'

const sample: RegisteredTriggerSummary[] = [
  {
    id: 'trg_9f3a2b1c44',
    worker_name: 'harness',
    trigger_type: 'harness::turn-completed',
    function_id: 'harness::react',
    config_summary: '{"parent_session_id":"s_root"}',
  },
  {
    id: 'trg_11aa22bb33',
    worker_name: 'engine',
    trigger_type: 'cron',
    function_id: 'harness::sweep_pending',
    config_summary: 'every 60s',
  },
]

const meta = {
  title: 'Pages/TriggersList',
  component: TriggersList,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof TriggersList>

export default meta
type Story = StoryObj<typeof meta>

export const Populated: Story = {
  args: { triggers: sample },
}

export const Empty: Story = { args: { triggers: [] } }
