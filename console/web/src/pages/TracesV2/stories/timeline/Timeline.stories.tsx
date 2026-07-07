/**
 * Live timeline lab. Every story (except Empty) drives the Timeline with the
 * simulated real-time span feed from ./liveFeed: history for the past ~70s
 * is pre-seeded so the window is full at mount, then a 200ms scheduler keeps
 * spawning spans, completing them at their planned end, and pruning ones
 * that scrolled out. Colors come from the production path — each label
 * hashes into WORKER_PALETTE, and wide-enough bars reveal the span name
 * with a leading ellipsis.
 *
 * Motion: the track slides on a virtual clock — it tracks the wall clock
 * while anything is running, PARKS at the last span's end when the feed
 * goes quiet (FreezesWhenIdle), and whooshes the gap in when the next span
 * arrives. Push past 4 concurrent spans (Bursts / Overloaded) to see
 * overflow collapse into icon avatar-stacks on the longest running bar.
 *
 * Interactions: hover any bar/chip for the span hover-card; click to
 * select (2px accent ring) — the selection stays glued to its bar while
 * the track slides.
 */

import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { Timeline } from '../../components/timeline/Timeline'
import { LabFrame } from '../harness'
import {
  BURSTS,
  IDLE_GAPS,
  LONG_RUNNING,
  MIXED,
  OVERLOADED,
  type Scenario,
  SPARSE,
  STEADY,
  useLiveSpans,
  WITH_ERRORS,
} from './liveFeed'

function LiveTimeline({ scenario }: { scenario: Scenario }) {
  const spans = useLiveSpans(scenario)
  const [selectedId, setSelectedId] = useState<string | undefined>(undefined)
  return (
    <Timeline
      spans={spans}
      onSpanClick={(span) =>
        setSelectedId((prev) => (prev === span.id ? undefined : span.id))
      }
      selectedSpanId={selectedId}
    />
  )
}

const meta = {
  title: 'TracesV2/Timeline',
  component: Timeline,
  parameters: { layout: 'centered' },
  decorators: [
    (Story) => (
      <LabFrame className="h-[240px] w-[840px]">
        <Story />
      </LabFrame>
    ),
  ],
  args: { spans: [] },
} satisfies Meta<typeof Timeline>

export default meta
type Story = StoryObj<typeof meta>

/** Moderate traffic — the default feel. */
export const SteadyTraffic: Story = {
  render: () => <LiveTimeline scenario={STEADY} />,
}

/** One span every 6–12s, hugging the center axis. */
export const Sparse: Story = {
  render: () => <LiveTimeline scenario={SPARSE} />,
}

/**
 * ~30s of dead air between clumps: the track parks at the last span's end
 * (nothing moves), then whooshes the gap in when the next clump arrives.
 */
export const FreezesWhenIdle: Story = {
  render: () => <LiveTimeline scenario={IDLE_GAPS} />,
}

/** Calm baseline with a 9-span clump every 14s — watch stacks form and clear. */
export const Bursts: Story = {
  render: () => <LiveTimeline scenario={BURSTS} />,
}

/** Permanently past 4 concurrent spans: avatar stacks everywhere. */
export const Overloaded: Story = {
  render: () => <LiveTimeline scenario={OVERLOADED} />,
}

/** 15–40s spans — most bars are live and visibly growing at the right edge. */
export const LongRunning: Story = {
  render: () => <LiveTimeline scenario={LONG_RUNNING} />,
}

/** Dense feed cycling all four icons across the full palette. */
export const MixedKindsAndColors: Story = {
  render: () => <LiveTimeline scenario={MIXED} />,
}

/** A 35% error rate — failures flip to alert red. */
export const WithErrors: Story = {
  render: () => <LiveTimeline scenario={WITH_ERRORS} />,
}

/** Six lanes instead of the default four — the pre-strip density. */
export const SixLanes: Story = {
  render: function SixLanesStory() {
    const spans = useLiveSpans(OVERLOADED)
    return <Timeline spans={spans} maxLanes={6} />
  },
}

/** No spans at all: just the axis and the 15s grid sliding by. */
export const Empty: Story = {}
