/**
 * The TracesV2 masthead: the live Timeline wearing the page-header chrome
 * (header row with the `$ traces` eyebrow + live/paused state on the
 * left, system/pause/refresh actions on the right).
 *
 * Every story is a stateful harness: the sim feed from ./liveFeed is
 * converted to `TraceListItem`s (one bar per trace, chromatic service
 * colors — exactly what production data looks like), pause actually
 * freezes the rows (the track parks at the last span once everything
 * completes), and clicking a bar selects it — in the app that opens the
 * trace's full-canvas detail.
 */

import type { Meta, StoryObj } from '@storybook/react-vite'
import { useEffect, useMemo, useRef, useState } from 'react'
import type { TimelineSpan } from '../../components/timeline/Timeline'
import { TimelineStrip } from '../../components/timeline/TimelineStrip'
import type { TraceListItem } from '../../hooks/useTraceData'
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

/** Shape the sim spans like the trace list rows the strip consumes. */
function toTraceListItems(spans: readonly TimelineSpan[]): TraceListItem[] {
  return spans.map((s) => ({
    traceId: s.id,
    rootOperation: s.label ?? s.id,
    functionId: s.kind !== 'zap' ? s.label : undefined,
    topic: s.kind === 'zap' ? `${s.label}.queue` : undefined,
    status:
      s.status === 'error' ? 'error' : s.endTime == null ? 'pending' : 'ok',
    startTime: s.startTime,
    endTime: s.endTime ?? undefined,
    duration: s.endTime != null ? s.endTime - s.startTime : undefined,
    spanCount: 1,
    services: [`worker-${s.label ?? 'sim'}`],
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
  const [isPaused, setIsPaused] = useState(initialPaused)
  const [showSystem, setShowSystem] = useState(false)
  const [selected, setSelected] = useState<string | null>(null)
  const spans = usePausableSpans(scenario, isPaused)
  const traces = useMemo(() => toTraceListItems(spans), [spans])
  return (
    <TimelineStrip
      traces={traces}
      isPaused={isPaused}
      showSystem={showSystem}
      onTogglePause={() => setIsPaused((v) => !v)}
      onToggleSystem={() => setShowSystem((v) => !v)}
      onRefresh={() => {}}
      onTraceClick={(traceId) =>
        setSelected((prev) => (prev === traceId ? null : traceId))
      }
      selectedTraceId={selected}
    />
  )
}

/**
 * The wire-faithful harness: rows are instant enqueue ROOTS (all the engine's
 * root-only rows stream ever delivers — start ≈ end, no pending status), and
 * liveness comes exclusively from the span-close `activity` map, exactly like
 * production. Each trace materializes as a dot at its birth, goes live once
 * child closes outrun the echo dead-band, and settles at its real end when
 * the activity stops. Pausing freezes the activity feed — bars settle within
 * the idle threshold, mirroring the real pause semantics.
 */
function ProductionShapeHarness({ scenario }: { scenario: Scenario }) {
  const [isPaused, setIsPaused] = useState(false)
  const [showSystem, setShowSystem] = useState(false)
  const [selected, setSelected] = useState<string | null>(null)
  const spans = usePausableSpans(scenario, isPaused)

  // The root row: a ~40ms "publish" moment. Status is settled at close (the
  // engine never sends pending rows); errors ride the root's own status.
  const traces = useMemo(
    () =>
      spans.map(
        (s): TraceListItem => ({
          traceId: s.id,
          rootOperation: `publish ${s.label ?? s.id}`,
          topic: `${s.label}.queue`,
          status: s.status === 'error' ? 'error' : 'ok',
          startTime: s.startTime,
          endTime: s.startTime + 40,
          duration: 40,
          spanCount: 1,
          services: [`worker-${s.label ?? 'sim'}`],
        }),
      ),
    [spans],
  )

  // Span-close activity: a running trace keeps closing children (~every
  // second here); a finished trace's last close was its end. Paused feeds
  // freeze, so live bars decay and settle like production.
  const [activityNow, setActivityNow] = useState(() => Date.now())
  useEffect(() => {
    if (isPaused) return
    const id = setInterval(() => setActivityNow(Date.now()), 1_000)
    return () => clearInterval(id)
  }, [isPaused])
  const activity = useMemo(() => {
    const map = new Map<string, number>()
    for (const s of spans) map.set(s.id, s.endTime ?? activityNow)
    return map
  }, [spans, activityNow])

  return (
    <TimelineStrip
      traces={traces}
      activity={activity}
      isPaused={isPaused}
      showSystem={showSystem}
      onTogglePause={() => setIsPaused((v) => !v)}
      onToggleSystem={() => setShowSystem((v) => !v)}
      onRefresh={() => {}}
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
    traces: [],
    isPaused: false,
    showSystem: false,
    onTogglePause: () => {},
    onToggleSystem: () => {},
    onRefresh: () => {},
  },
} satisfies Meta<typeof TimelineStrip>

export default meta
type Story = StoryObj<typeof meta>

/** Moderate traffic — one chromatic bar per trace. */
export const SteadyTraffic: Story = {
  render: () => <StripHarness scenario={STEADY} />,
}

/**
 * What the engine actually sends: instant root rows + span-close activity.
 * Dots at birth grow into live bars while children close, then settle at
 * the trace's real end.
 */
export const ProductionShape: Story = {
  render: () => <ProductionShapeHarness scenario={STEADY} />,
}

/** A quiet engine: one trace every 6–12s hugging the axis. */
export const Sparse: Story = {
  render: () => <StripHarness scenario={SPARSE} />,
}

/** 9-trace clumps every 14s — overflow fans into chip stacks. */
export const Bursts: Story = {
  render: () => <StripHarness scenario={BURSTS} />,
}

/** Permanently past 4 concurrent traces: stacks ride the longest bar. */
export const Overloaded: Story = {
  render: () => <StripHarness scenario={OVERLOADED} />,
}

/** 35% error rate — failed traces flip to alert red. */
export const WithErrors: Story = {
  render: () => <StripHarness scenario={WITH_ERRORS} />,
}

/** Rows frozen (warn badge up), while the window keeps sliding. */
export const Paused: Story = {
  render: () => <StripHarness scenario={STEADY} initialPaused />,
}

/** No traces at all: just the chrome and the grid drifting by. */
export const Empty: Story = {}
