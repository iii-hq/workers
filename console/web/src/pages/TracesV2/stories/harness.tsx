/**
 * Storybook harness for the TracesV2 lab.
 *
 * Two concerns live here:
 *
 * 1. A FAKE iii-client, installed through the real `__setIiiClientDepsForTests`
 *    seam in `@/lib/iii-client`. The seam lets us swap the `registerWorker`
 *    dependency for one that returns a fake `ISdk`; `getIiiClient()` then hands
 *    every fetch (`engine::traces::list`, `engine::logs::list`, …) a canned
 *    response built from the fixtures — no WebSocket, no engine. Stream
 *    subscriptions (`registerFunction` / `registerTrigger`) are accepted and
 *    ignored, so the live-append effects simply never fire (static playground).
 *
 * 2. `LabFrame` — a bounded, themed container. Several trace components
 *    (WaterfallChart's react-virtual list, the timeline lanes) render
 *    NOTHING at zero height, so every story that mounts one must give it a
 *    real height. Use `LabFrame` (or `fullscreen` + `h-screen`) to guarantee it.
 */

import type { Decorator } from '@storybook/react-vite'
import type { ISdk } from 'iii-browser-sdk'
import { type ReactNode, useEffect, useState } from 'react'
import {
  __resetIiiClientForTests,
  __setIiiClientDepsForTests,
} from '@/lib/iii-client'
import type { SpanTreeNode, StoredSpan, TraceSummary } from '../api/traces'
import {
  ALL_SPANS,
  LIST_SPANS,
  OTEL_LOGS_FIXTURE,
  T0,
  TRACE_GROUPS_FIXTURE,
} from '../fixtures/traces-fixtures'

type TriggerArgs = { function_id: string; payload?: Record<string, unknown> }

/** Nest a flat span list into the `{ roots }` tree shape `engine::traces::tree` returns. */
function buildTraceTree(traceId: string): SpanTreeNode[] {
  const spans = ALL_SPANS.filter((s) => s.trace_id === traceId)
  const nodes = new Map<string, SpanTreeNode>()
  for (const s of spans) nodes.set(s.span_id, { ...s, children: [] })
  const roots: SpanTreeNode[] = []
  for (const s of spans) {
    const node = nodes.get(s.span_id) as SpanTreeNode
    const parent = s.parent_span_id ? nodes.get(s.parent_span_id) : undefined
    if (parent) parent.children.push(node)
    else roots.push(node)
  }
  return roots
}

function toSummary(root: StoredSpan): TraceSummary {
  const spans = ALL_SPANS.filter((span) => span.trace_id === root.trace_id)
  const errors = spans.filter(
    (span) => span.status.toLowerCase() === 'error',
  ).length
  const pending = spans.some((span) => span.pending)
  const attributes = Object.fromEntries(
    root.attributes.map(([key, value]) => [key, String(value)]),
  )
  return {
    trace_id: root.trace_id,
    name: root.name,
    start_time_unix_nano: Math.min(
      ...spans.map((span) => span.start_time_unix_nano),
    ),
    ...(pending
      ? {}
      : {
          end_time_unix_nano: Math.max(
            ...spans.map((span) => span.end_time_unix_nano),
          ),
        }),
    status: errors > 0 ? 'error' : pending ? 'pending' : 'ok',
    service_name: root.service_name,
    function_id: String(
      attributes['faas.invoked_name'] ?? attributes.function_id ?? '',
    ),
    topic: attributes['messaging.destination.name'] as string | undefined,
    trace_tags: root.trace_tags,
    attributes,
    span_count: spans.length,
    error_count: errors,
  }
}

/** Route a bus trigger to a canned, fixture-backed response. */
async function fakeTrigger({
  function_id,
  payload,
}: TriggerArgs): Promise<unknown> {
  const p = payload ?? {}
  switch (function_id) {
    case 'engine::traces::list': {
      // The list read returns summaries, with lightweight filter/search support so
      // the filter bar and search box visibly do something in the playground.
      let traces = LIST_SPANS.map(toSummary)
      const name = p.name as string | undefined
      const worker = p.service_name as string | undefined
      const status = p.status as string | undefined
      if (name) {
        const q = name.toLowerCase()
        traces = traces.filter((trace) => trace.name.toLowerCase().includes(q))
      }
      if (worker)
        traces = traces.filter((trace) => trace.service_name === worker)
      if (status) {
        traces = traces.filter((trace) => trace.status === status)
      }
      const excluded = p.exclude_attributes as
        | Array<[string, string]>
        | undefined
      if (excluded?.length) {
        traces = traces.filter(
          (trace) =>
            !excluded.some(([key, value]) => {
              if (key === 'faas.invoked_name' || key === 'function_id') {
                return trace.function_id === value
              }
              return trace.attributes?.[key] === value
            }),
        )
      }
      const total = traces.length
      const offset = (p.offset as number) ?? 0
      const limit = (p.limit as number) ?? 50
      return {
        traces: traces.slice(offset, offset + limit),
        total,
        offset,
        limit,
      }
    }
    case 'engine::traces::spans': {
      const traceId = p.trace_id as string | undefined
      const spans = traceId
        ? ALL_SPANS.filter((span) => span.trace_id === traceId)
        : ALL_SPANS
      return { spans, total: spans.length, offset: 0, limit: 10000 }
    }
    case 'engine::traces::tree':
      return { roots: buildTraceTree((p.trace_id as string) ?? '') }
    case 'engine::traces::group_by':
      return { groups: TRACE_GROUPS_FIXTURE }
    case 'engine::logs::list': {
      const spanId = p.span_id as string | undefined
      const traceId = p.trace_id as string | undefined
      let logs = OTEL_LOGS_FIXTURE
      if (spanId) logs = logs.filter((l) => l.span_id === spanId)
      else if (traceId) logs = logs.filter((l) => l.trace_id === traceId)
      return { logs, total: logs.length, timestamp: T0 }
    }
    case 'engine::traces::clear':
    case 'engine::logs::clear':
      return { success: true }
    default:
      return {}
  }
}

const noopRef = { unregister() {} }

/** A minimal `ISdk` that satisfies everything `wrapSdk` in iii-client.ts touches. */
function makeFakeSdk(): ISdk {
  const fake = {
    trigger: (args: TriggerArgs) => fakeTrigger(args),
    registerFunction: () => noopRef,
    registerTrigger: () => noopRef,
    // Never emit a state so the reseed-on-connect effect stays quiet.
    addConnectionStateListener: () => () => {},
    shutdown: async () => {},
  }
  return fake as unknown as ISdk
}

/** Install the fake client (wipes the cached singleton so the next call rebuilds). */
export function installFakeIiiClient(): void {
  __setIiiClientDepsForTests({ registerWorker: () => makeFakeSdk() })
}

/** Restore the real client dependencies. */
export function resetFakeIiiClient(): void {
  __resetIiiClientForTests()
}

/**
 * Decorator: install the fake client before the story's components mount, and
 * restore it on unmount. The `useState` lazy initializer runs synchronously
 * during this wrapper's render — i.e. before the wrapped `<Story/>` renders and
 * calls `getIiiClient()`.
 */
export const withFakeIiiClient: Decorator = (Story) => {
  useState(() => {
    installFakeIiiClient()
    return null
  })
  useEffect(() => resetFakeIiiClient, [])
  return <Story />
}

/**
 * A bounded, themed frame for stories whose components need a real height.
 * Defaults to a comfortable panel size; pass `className` to override.
 */
export function LabFrame({
  children,
  className = 'h-[560px] w-full max-w-[560px]',
}: {
  children: ReactNode
  className?: string
}) {
  return (
    <div
      className={`flex flex-col overflow-hidden border border-rule bg-bg ${className}`}
    >
      {children}
    </div>
  )
}
