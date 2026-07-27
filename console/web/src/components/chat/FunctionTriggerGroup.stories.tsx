import type { Meta, StoryObj } from '@storybook/react-vite'
import type { FunctionTriggerMessage as FTriggerType } from '@/types/chat'
import { FunctionTriggerGroup } from './FunctionTriggerGroup'

/* Mirrors the multi-function-agent scenario: three sequential engine calls
   so the story matches what a real fan-out turn produces. */
const groupInFlight: FTriggerType[] = [
  {
    id: 'g1a',
    role: 'function-trigger',
    functionId: 'engine::list',
    input: {},
    output: { workers: ['worker-1', 'worker-3', 'worker-7'] },
    durationMs: 450,
    createdAt: Date.now(),
  },
  {
    id: 'g1b',
    role: 'function-trigger',
    functionId: 'engine::info',
    input: { id: 'worker-7' },
    running: true,
    createdAt: Date.now(),
  },
  {
    id: 'g1c',
    role: 'function-trigger',
    functionId: 'engine::echo',
    input: { workerId: 'worker-7', text: 'ping' },
    output: { text: 'ping' },
    durationMs: 350,
    createdAt: Date.now(),
  },
]

const groupDone: FTriggerType[] = [
  {
    id: 'g2a',
    role: 'function-trigger',
    functionId: 'engine::list',
    input: {},
    output: { workers: ['worker-1', 'worker-3', 'worker-7'] },
    durationMs: 450,
    createdAt: Date.now(),
  },
  {
    id: 'g2b',
    role: 'function-trigger',
    functionId: 'engine::info',
    input: { id: 'worker-7' },
    output: {
      id: 'worker-7',
      load: 0.12,
      version: '0.4.1',
      skills: ['echo', 'tokenize', 'embed'],
    },
    durationMs: 500,
    createdAt: Date.now(),
  },
  {
    id: 'g2c',
    role: 'function-trigger',
    functionId: 'engine::echo',
    input: { workerId: 'worker-7', text: 'ping' },
    output: { text: 'ping' },
    durationMs: 350,
    createdAt: Date.now(),
  },
]

const meta = {
  title: 'Chat/FunctionTriggerGroup',
  component: FunctionTriggerGroup,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof FunctionTriggerGroup>

export default meta
type Story = StoryObj<typeof meta>

export const InFlight: Story = {
  name: 'in-flight (3 functions, 2nd triggering)',
  args: { messages: groupInFlight },
}

export const DoneCollapsed: Story = {
  name: 'triggered (3 functions, collapsed)',
  args: { messages: groupDone },
}

export const DoneExpanded: Story = {
  name: 'triggered (3 functions, expanded)',
  args: { messages: groupDone, defaultOpen: true },
}
