import type { Meta, StoryObj } from '@storybook/react-vite'
import { TRACE_1_ID, TRACE_2_ID } from '../fixtures/traces-fixtures'
import { TracesV2 } from '../index'
import { withFakeIiiClient } from './harness'

/**
 * The full TracesV2 surface, wired end-to-end against the fake iii-client.
 *
 * The live TimelineStrip masthead up top (fixture traces are in the past,
 * so the strip mostly shows the grid sliding by), the filter bar, and the
 * trace list. Click a row (or a strip bar) to expand the trace's detail
 * ACCORDION-style right beneath it — clicking another row swaps the
 * expansion, clicking the open row (or its ✕, or Esc) collapses it. The
 * view switcher offers the lane timeline (default) and the waterfall;
 * click a span to open the right-hand span panel; Esc walks back out.
 * This is the interactive playground — iterate on the whole composition
 * here; iterate on pieces in the per-component stories.
 */
const meta = {
  title: 'TracesV2/Playground',
  component: TracesV2,
  parameters: { layout: 'fullscreen' },
  decorators: [withFakeIiiClient],
} satisfies Meta<typeof TracesV2>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {
  render: () => (
    <div className="h-screen flex flex-col bg-bg text-ink font-sans">
      <TracesV2 />
    </div>
  ),
}

/**
 * Mounted with the rich agent trace already expanded: the accordion detail
 * with the lane timeline view active (4 nested ancestors as bars + 3 leaf
 * chips). Click a bar/chip to open the span panel; drag the separator to
 * resize it.
 */
export const DetailFullCanvas: Story = {
  render: () => (
    <div className="h-screen flex flex-col bg-bg text-ink font-sans">
      <TracesV2 initialTraceId={TRACE_1_ID} />
    </div>
  ),
}

/** The two-span healthy trace — a sparse detail timeline. */
export const DetailSimpleTrace: Story = {
  render: () => (
    <div className="h-screen flex flex-col bg-bg text-ink font-sans">
      <TracesV2 initialTraceId={TRACE_2_ID} />
    </div>
  ),
}
