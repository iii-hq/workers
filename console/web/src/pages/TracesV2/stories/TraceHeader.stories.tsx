import type { Meta, StoryObj } from '@storybook/react-vite'
import { fn } from 'storybook/test'
import { TraceHeader } from '../components/TraceHeader'
import {
  TRACE_1_ID,
  TRACE_2_ID,
  WATERFALL_FIXTURE,
  WATERFALL_SIMPLE,
} from '../fixtures/traces-fixtures'

const meta = {
  title: 'TracesV2/TraceHeader',
  component: TraceHeader,
  parameters: { layout: 'centered' },
  decorators: [
    (Story) => (
      <div className="w-[560px] border border-rule bg-panel">
        <Story />
      </div>
    ),
  ],
  args: {
    data: WATERFALL_FIXTURE,
    traceId: TRACE_1_ID,
    onClose: fn(),
    onSpanClick: fn(),
  },
} satisfies Meta<typeof TraceHeader>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {}

export const SimpleTrace: Story = {
  args: { data: WATERFALL_SIMPLE, traceId: TRACE_2_ID },
}
