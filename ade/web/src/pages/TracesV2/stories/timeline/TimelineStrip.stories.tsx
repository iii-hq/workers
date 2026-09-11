/**
 * The TracesV2 masthead: the live hierarchical Timeline wearing the
 * page-header chrome (header row with the `$ traces` eyebrow + live/paused
 * state on the left, the shared span-filter funnel on the right).
 *
 * Every story is a stateful harness: the sim feed from ./liveFeed is
 * threaded into parent chains and converted to `StoredSpan`s (exactly the
 * all-spans feed the strip consumes in production — one 3px line per span,
 * live bars while a sim span is still running), the Paused story freezes
 * the feed (the track parks at the last span once everything completes),
 * and clicking a bar selects its trace — in the app that opens the trace's
 * full-canvas detail.
 */

import type { Meta, StoryObj } from '@storybook/react-vite'
import { useMemo, useRef, useState } from 'react'
import type { StoredSpan } from '../../api/traces'
import type { TimelineSpan } from '../../components/timeline/layout'
import { TimelineStrip } from '../../components/timeline/TimelineStrip'
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

function hashId(id: string): number {
  let h = 5381
  for (let i = 0; i < id.length; i++) {
    h = ((h << 5) + h + id.charCodeAt(i)) >>> 0
  }
  return h
}

/**
 * Thread the flat sim feed into parent chains for the strip's hierarchy:
 * ~70% of spans (gated by an id hash) attach to the NEAREST span that
 * started 0.15–8s earlier (depth-capped at 4) and inherit its trace id.
 * Everything derives from immutable ids and start times, so the tree is
 * stable across feed ticks — no bar ever changes parent as spans settle
 * or arrive.
 */
function toHierarchicalStoredSpans(
  spans: readonly TimelineSpan[],
): StoredSpan[] {
  const MIN_GAP_NS = 150 * 1_000_000
  const MAX_GAP_NS = 8_000 * 1_000_000
  const stored = toStoredSpans(spans)
  const sorted = [...stored].sort(
    (a, b) => a.start_time_unix_nano - b.start_time_unix_nano,
  )
  const depth = new Map<string, number>()
  for (let i = 0; i < sorted.length; i++) {
    const span = sorted[i]
    if (hashId(span.span_id) % 100 >= 70) continue
    for (let j = i - 1; j >= 0; j--) {
      const parent = sorted[j]
      const gap = span.start_time_unix_nano - parent.start_time_unix_nano
      if (gap < MIN_GAP_NS) continue
      if (gap > MAX_GAP_NS) break
      const parentDepth = depth.get(parent.span_id) ?? 0
      if (parentDepth >= 4) continue
      span.parent_span_id = parent.span_id
      span.trace_id = parent.trace_id
      depth.set(span.span_id, parentDepth + 1)
      break
    }
  }
  return stored
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
  // The strip doesn't render selection — this just mirrors the app's
  // click contract (toggle a trace's full-canvas detail open/closed).
  const [selected, setSelected] = useState<string | null>(null)
  const simSpans = usePausableSpans(scenario, isPaused)
  const spans = useMemo(() => toHierarchicalStoredSpans(simSpans), [simSpans])
  return (
    <TimelineStrip
      spans={spans}
      isPaused={isPaused}
      onTraceClick={(traceId) =>
        setSelected(selected === traceId ? null : traceId)
      }
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

/** Dense mixed feed — parents above what they triggered, elbow connectors,
 *  sequential subtrees sharing lines flame-graph style. */
export const Hierarchy: Story = {
  render: () => <StripHarness scenario={MIXED} />,
}

/** Moderate traffic — the default feel. */
export const SteadyTraffic: Story = {
  render: () => <StripHarness scenario={STEADY} />,
}

/** A quiet engine: one span every 6–12s, mostly roots on the top lines. */
export const Sparse: Story = {
  render: () => <StripHarness scenario={SPARSE} />,
}

/** 9-span clumps every 14s: concurrent subtrees stack downward, then the
 *  lines clear as the clump scrolls out. */
export const Bursts: Story = {
  render: () => <StripHarness scenario={BURSTS} />,
}

/** Permanently deep concurrency — the viewport scrolls vertically when
 *  the hierarchy outgrows the strip. */
export const Overloaded: Story = {
  render: () => <StripHarness scenario={OVERLOADED} />,
}

/** 15–40s spans — most bars are live, growing width-only along the
 *  now-edge (the hierarchy owns the left edge). */
export const LongRunning: Story = {
  render: () => <StripHarness scenario={LONG_RUNNING} />,
}

/** 35% error rate — failed spans flip to alert red. */
export const WithErrors: Story = {
  render: () => <StripHarness scenario={WITH_ERRORS} />,
}

/**
 * ~30s of dead air between clumps: the track parks at the last span's end
 * (nothing moves), then whooshes the gap in when the next clump arrives.
 */
export const FreezesWhenIdle: Story = {
  render: () => <StripHarness scenario={IDLE_GAPS} />,
}

/** Feed frozen (warn badge up), while the window keeps sliding. */
export const Paused: Story = {
  render: () => <StripHarness scenario={STEADY} initialPaused />,
}

/** No spans at all: just the chrome and the grid drifting by. */
export const Empty: Story = {}
