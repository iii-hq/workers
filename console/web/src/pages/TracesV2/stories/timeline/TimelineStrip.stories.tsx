/**
 * The TracesV2 masthead: the live Timeline wearing the page-header chrome
 * (header row with the `$ traces` eyebrow + live/paused state on the
 * left, the shared span-filter funnel on the right).
 *
 * Every story is a stateful harness: the sim feed from ./liveFeed is
 * converted to `StoredSpan`s (one bar per span — exactly the all-spans
 * feed the strip consumes in production, live bars while a sim span is
 * still running), the Paused story freezes the feed (the track parks at
 * the last span once everything completes), and clicking a bar selects
 * its trace — in the app that opens the trace's full-canvas detail.
 */

import type { Meta, StoryObj } from '@storybook/react-vite'
import { useMemo, useRef, useState } from 'react'
import type { StoredSpan } from '../../api/traces'
import type { TimelineSpan } from '../../components/timeline/Timeline'
import { TimelineStrip } from '../../components/timeline/TimelineStrip'
import { LabFrame } from '../harness'
import {
  BURSTS,
  OVERLOADED,
  type Scenario,
  SPARSE,
  STEADY,
  useLiveSpans,
  WITH_ERRORS,
} from './liveFeed'

/** Shape the sim spans like the stored spans the strip consumes. */
function toStoredSpans(spans: readonly TimelineSpan[]): StoredSpan[] {
  return spans.map((s) => ({
    trace_id: `trace-${s.id}`,
    span_id: s.id,
    name: `execute ${s.label ?? s.id}`,
    kind:
      s.kind === 'zap'
        ? 'server'
        : s.kind === 'flame'
          ? 'consumer'
          : s.kind === 'sparkle'
            ? 'client'
            : 'internal',
    start_time_unix_nano: s.startTime * 1_000_000,
    // A still-running sim span is a pending live snapshot (end 0).
    end_time_unix_nano: s.endTime == null ? 0 : s.endTime * 1_000_000,
    status: s.status === 'error' ? 'ERROR' : 'OK',
    attributes: [['function_id', s.label ?? 'sim']],
    events: [],
    links: [],
    service_name: `worker-${s.label ?? 'sim'}`,
  }))
}

/** While paused, render the last live snapshot (the feed keeps simulating). */
function usePausableSpans(scenario: Scenario, paused: boolean): TimelineSpan[] {
  const live = useLiveSpans(scenario)
  const lastLiveRef = useRef<TimelineSpan[]>([])
  if (!paused) lastLiveRef.current = live
  return paused ? lastLiveRef.current : live
}

function StripHarness({
  scenario,
  initialPaused = false,
}: {
  scenario: Scenario
  initialPaused?: boolean
}) {
  const [isPaused] = useState(initialPaused)
  const [selected, setSelected] = useState<string | null>(null)
  const simSpans = usePausableSpans(scenario, isPaused)
  const spans = useMemo(() => toStoredSpans(simSpans), [simSpans])
  return (
    <TimelineStrip
      spans={spans}
      isPaused={isPaused}
      onTraceClick={(traceId) =>
        setSelected((prev) => (prev === traceId ? null : traceId))
      }
      selectedTraceId={selected}
    />
  )
}

const meta = {
  title: 'TracesV2/TimelineStrip',
  component: TimelineStrip,
  parameters: { layout: 'centered' },
  decorators: [
    (Story) => (
      <LabFrame className="w-[960px]">
        <Story />
      </LabFrame>
    ),
  ],
  args: {
    spans: [],
    isPaused: false,
  },
} satisfies Meta<typeof TimelineStrip>

export default meta
type Story = StoryObj<typeof meta>

/** Moderate traffic — one chromatic bar per span. */
export const SteadyTraffic: Story = {
  render: () => <StripHarness scenario={STEADY} />,
}

/** A quiet engine: one span every 6–12s hugging the axis. */
export const Sparse: Story = {
  render: () => <StripHarness scenario={SPARSE} />,
}

/** 9-span clumps every 14s — overflow fans into chip stacks. */
export const Bursts: Story = {
  render: () => <StripHarness scenario={BURSTS} />,
}

/** Permanently past 4 concurrent spans: stacks ride the longest bar. */
export const Overloaded: Story = {
  render: () => <StripHarness scenario={OVERLOADED} />,
}

/** 35% error rate — failed spans flip to alert red. */
export const WithErrors: Story = {
  render: () => <StripHarness scenario={WITH_ERRORS} />,
}

/** Feed frozen (warn badge up), while the window keeps sliding. */
export const Paused: Story = {
  render: () => <StripHarness scenario={STEADY} initialPaused />,
}

/** No spans at all: just the chrome and the grid drifting by. */
export const Empty: Story = {}
