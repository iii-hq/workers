/**
 * The static trace-scoped timeline — the detail view that replaced the
 * flame graph. Unlike the live strip's thread lanes, this one is
 * hierarchical: each child sits on a line below its parent, inset a
 * small padding from the parent's left edge with elbow connectors, so
 * you can read exactly which span started what. Sequential siblings
 * share a single line (flame-graph style); only concurrent subtrees
 * stack onto extra lines. Every span is always visible (no chip
 * collapsing); tall traces pan vertically by mouse drag.
 *
 * The rich agent fixture shows a 4-deep nested root chain whose three
 * leaves (llm / tool / db) run sequentially — one shared line. Hover
 * anything for the span hover-card (with % of trace); click to select.
 */

import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { fn } from 'storybook/test'
import type { StoredSpan } from '../../api/traces'
import { TraceTimeline } from '../../components/timeline/TraceTimeline'
import {
  ERROR_SPAN,
  T0,
  TRACE_3_ID,
  TRACE_3_SPANS,
  WATERFALL_FIXTURE,
  WATERFALL_SIMPLE,
} from '../../fixtures/traces-fixtures'
import {
  toWaterfallData,
  type VisualizationSpan,
  type WaterfallData,
} from '../../lib/traceTransform'
import { LabFrame } from '../harness'

const WATERFALL_ERROR = toWaterfallData(
  TRACE_3_SPANS,
  TRACE_3_ID,
) as WaterfallData

/**
 * A synthetic fan-out trace: a root running 6 sequential stages (they
 * share ONE line), each stage bursting 4 CONCURRENT workers that each
 * make a db call. The concurrency is what stacks lines — enough to
 * outgrow the 260px frame and exercise the drag-to-pan scrolling.
 */
const DEEP_TRACE_ID = 'trace-deep-0000000000000009'
const DEEP_TRACE_SPANS: StoredSpan[] = (() => {
  const spans: StoredSpan[] = [
    {
      trace_id: DEEP_TRACE_ID,
      span_id: 'root',
      name: 'pipeline.run',
      kind: 'server',
      service_name: 'orchestrator',
      start_time_unix_nano: T0,
      end_time_unix_nano: T0 + 3000,
      status: 'OK',
      attributes: [],
      events: [],
      links: [],
    },
  ]
  for (let stage = 0; stage < 6; stage++) {
    const stageStart = T0 + 40 + stage * 480
    spans.push({
      trace_id: DEEP_TRACE_ID,
      span_id: `stage-${stage}`,
      parent_span_id: 'root',
      name: `stage.${stage}.execute`,
      kind: 'internal',
      service_name: `worker-${stage % 3}`,
      start_time_unix_nano: stageStart,
      end_time_unix_nano: stageStart + 440,
      status: 'OK',
      attributes: [],
      events: [],
      links: [],
    })
    for (let step = 0; step < 4; step++) {
      // All four workers of a stage start together: truly concurrent.
      const stepStart = stageStart + 20 + step * 5
      spans.push({
        trace_id: DEEP_TRACE_ID,
        span_id: `stage-${stage}-step-${step}`,
        parent_span_id: `stage-${stage}`,
        name: `worker.${step}`,
        kind: 'internal',
        service_name: `worker-${stage % 3}`,
        start_time_unix_nano: stepStart,
        end_time_unix_nano: stepStart + 380 - step * 10,
        status: 'OK',
        attributes: [],
        events: [],
        links: [],
      })
      spans.push({
        trace_id: DEEP_TRACE_ID,
        span_id: `stage-${stage}-step-${step}-db`,
        parent_span_id: `stage-${stage}-step-${step}`,
        name: 'db.write',
        kind: 'client',
        service_name: 'postgres',
        start_time_unix_nano: stepStart + 40,
        end_time_unix_nano: stepStart + 140,
        status: 'OK',
        attributes: [],
        events: [],
        links: [],
      })
    }
  }
  return spans
})()
const WATERFALL_DEEP = toWaterfallData(
  DEEP_TRACE_SPANS,
  DEEP_TRACE_ID,
) as WaterfallData

const meta = {
  title: 'TracesV2/TraceTimeline',
  component: TraceTimeline,
  parameters: { layout: 'centered' },
  decorators: [
    (Story) => (
      <LabFrame className="h-[260px] w-[840px]">
        <Story />
      </LabFrame>
    ),
  ],
  args: {
    data: WATERFALL_FIXTURE,
    onSpanClick: fn(),
  },
} satisfies Meta<typeof TraceTimeline>

export default meta
type Story = StoryObj<typeof meta>

/** The rich agent trace: a 4-deep cascade fanning into 3 leaf spans. */
export const RichTrace: Story = {}

/** Two quick spans across two services — a root and one child. */
export const SimpleTrace: Story = {
  args: { data: WATERFALL_SIMPLE },
}

/** Single errored queue consumer: one alert-red bar spanning the window. */
export const ErroredTrace: Story = {
  args: { data: WATERFALL_ERROR },
}

/** The errored tool span carries the accent selection ring. */
export const WithSelectedSpan: Story = {
  args: { selectedSpanId: ERROR_SPAN.span_id },
}

/** Concurrent fan-out stacks lines past the frame — drag anywhere to pan. */
export const DeepTraceDragToScroll: Story = {
  args: { data: WATERFALL_DEEP },
}

/** Stateful: click bars to move the selection, like in the detail. */
export const Selectable: Story = {
  render: function SelectableStory(args) {
    const [selected, setSelected] = useState<VisualizationSpan | null>(null)
    return (
      <TraceTimeline
        {...args}
        onSpanClick={(span) =>
          setSelected((prev) => (prev?.span_id === span.span_id ? null : span))
        }
        selectedSpanId={selected?.span_id}
      />
    )
  },
}

/**
 * The floating funnel expands on hover into the span filter menu: groups
 * ranked busiest-first (db.write's 24 calls top the list), checking one
 * hides its spans and their subtrees, and the window never rescales. Here
 * spans group by operation name; the traces page groups by owning
 * function id (`traceSpanGroupKey`).
 */
export const WithSpanFilterMenu: Story = {
  args: {
    data: WATERFALL_DEEP,
    spanGroupKey: (span) => span.name,
  },
}
