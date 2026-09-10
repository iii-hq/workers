import type { Meta, StoryObj } from '@storybook/react-vite'
import { TraceDetailSkeleton } from '../components/TraceDetailSkeleton'

/**
 * The trace-detail loading placeholder, rendered in a detail-shaped canvas.
 * Compare against Playground → DetailFullCanvas: every block (header, view
 * switcher, timeline, collapsed workers footer) should line up with the
 * loaded composition it stands in for.
 */
const meta = {
  title: 'TracesV2/TraceDetailSkeleton',
  component: TraceDetailSkeleton,
  parameters: { layout: 'fullscreen' },
  args: { onClose: () => {} },
  decorators: [
    (Story) => (
      <div className="h-screen flex flex-col bg-bg text-ink font-sans">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof TraceDetailSkeleton>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {}
