import type { Meta, StoryObj } from '@storybook/react-vite'
import type { CommEvent } from '@/types/iii-agent-event'
import { TimelineGrid } from './TimelineGrid'

const events: CommEvent[] = [
  {
    seq: 1, at: 1720000000000, root_session_id: 's_root', kind: 'spawn',
    from: { session_id: 's_root', turn_id: 't_1' },
    to: { session_id: 's_a', turn_id: 't_2' },
    summary: 'review the auth module', ref: { function_call_id: 'fc_1' },
  },
  {
    seq: 2, at: 1720000004000, root_session_id: 's_root', kind: 'spawn',
    from: { session_id: 's_a', turn_id: 't_2' },
    to: { session_id: 's_b', turn_id: 't_3' },
    summary: 'grep for insecure patterns', ref: { function_call_id: 'fc_2' },
  },
  {
    seq: 3, at: 1720000009000, root_session_id: 's_root', kind: 'trigger_fire',
    to: { session_id: 's_root' },
    trigger: { registered_trigger_id: 'trg_1', action: 'notify', label: 'build-done' },
    summary: '{"status":"green"}',
  },
  {
    seq: 4, at: 1720000009100, root_session_id: 's_root', kind: 'notify',
    to: { session_id: 's_root' },
    trigger: { registered_trigger_id: 'trg_1', action: 'notify', label: 'build-done' },
    summary: 'build-done',
  },
  {
    seq: 5, at: 1720000012000, root_session_id: 's_root', kind: 'result',
    from: { session_id: 's_b', turn_id: 't_3' },
    to: { session_id: 's_a', turn_id: 't_2' },
    summary: 'ok', ref: { function_call_id: 'fc_2' },
  },
  {
    seq: 6, at: 1720000015000, root_session_id: 's_root', kind: 'result',
    from: { session_id: 's_a', turn_id: 't_2' },
    to: { session_id: 's_root', turn_id: 't_1' },
    summary: 'ok', ref: { function_call_id: 'fc_1' },
  },
]

const meta = {
  title: 'Pages/TimelineGrid',
  component: TimelineGrid,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof TimelineGrid>

export default meta
type Story = StoryObj<typeof meta>

export const Family: Story = {
  args: {
    rootId: 's_root',
    lanes: ['s_root', 's_a', 's_b'],
    events,
    laneTitle: (id: string) =>
      ({ s_root: 'main agent', s_a: 'reviewer', s_b: 'grepper' })[id] ?? id,
    onOpenSession: () => {},
  },
}

export const Empty: Story = {
  args: { ...Family.args, events: [], lanes: ['s_root'] },
}
