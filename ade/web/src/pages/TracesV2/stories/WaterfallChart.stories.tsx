import type { Meta, StoryObj } from '@storybook/react-vite'
import { fn } from 'storybook/test'
import { WaterfallChart } from '../components/WaterfallChart'
import {
  ERROR_SPAN,
  WATERFALL_FIXTURE,
  WATERFALL_SIMPLE,
} from '../fixtures/traces-fixtures'
import { LabFrame } from './harness'

/**
 * The virtualized waterfall list. Needs a bounded, scrollable parent — the
 * `LabFrame` provides a fixed height; at zero height react-virtual renders no
 * rows. Toggle "hide engine routing" in the toolbar to collapse the
 * handle_invocation/call pair in the fixture.
 */
const meta = {
  title: 'TracesV2/WaterfallChart',
  component: WaterfallChart,
  parameters: { layout: 'centered' },
  decorators: [
    (Story) => (
      <LabFrame className="h-[560px] w-[760px]">
        <Story />
      </LabFrame>
    ),
  ],
  args: {
    data: WATERFALL_FIXTURE,
    onSpanClick: fn(),
  },
} satisfies Meta<typeof WaterfallChart>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {}

export const WithSelectedSpan: Story = {
  args: { selectedSpanId: ERROR_SPAN.span_id },
}

export const SimpleTrace: Story = {
  args: { data: WATERFALL_SIMPLE },
}
