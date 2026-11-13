import { TraceListRow } from '@iii-dev/traces-ui/src/components/TraceListRow'
import { TRACE_LIST_FIXTURE } from '@iii-dev/traces-ui/src/fixtures/traces-fixtures'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { fn } from 'storybook/test'

const HEALTHY_TRACE =
  TRACE_LIST_FIXTURE.find((t) => t.status === 'ok') ?? TRACE_LIST_FIXTURE[0]
const ERROR_TRACE =
  TRACE_LIST_FIXTURE.find((t) => t.status === 'error') ?? TRACE_LIST_FIXTURE[0]

const meta = {
  title: 'TracesV2/TraceListRow',
  component: TraceListRow,
  parameters: { layout: 'centered' },
  decorators: [
    (Story) => (
      <div className="w-[560px] border border-rule bg-bg">
        <Story />
      </div>
    ),
  ],
  args: {
    trace: HEALTHY_TRACE,
    isSelected: false,
    isNew: false,
    onSelect: fn(),
    onAnimationEnd: fn(),
  },
} satisfies Meta<typeof TraceListRow>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {}

export const Selected: Story = { args: { isSelected: true } }

export const New: Story = { args: { isNew: true } }

export const ErrorTrace: Story = { args: { trace: ERROR_TRACE } }

export const List: Story = {
  render: () => (
    <>
      {TRACE_LIST_FIXTURE.map((trace) => (
        <TraceListRow
          key={trace.traceId}
          trace={trace}
          isSelected={false}
          isNew={false}
          onSelect={fn()}
          onAnimationEnd={fn()}
        />
      ))}
    </>
  ),
}
