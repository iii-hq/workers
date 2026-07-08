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
import type { SpanTreeNode, StoredSpan } from '../api/traces'
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

/** Route a bus trigger to a canned, fixture-backed response. */
async function fakeTrigger({
  function_id,
  payload,
}: TriggerArgs): Promise<unknown> {
  const p = payload ?? {}
  switch (function_id) {
    case 'engine::traces::list': {
      const traceId = p.trace_id as string | undefined
      // A trace-scoped read (the detail seed) returns every span of that trace.
      if (traceId) {
        const spans = ALL_SPANS.filter((s) => s.trace_id === traceId)
        return { spans, total: spans.length, offset: 0, limit: 10000 }
      }
      // The list read returns roots, with lightweight filter/search support so
      // the filter bar and search box visibly do something in the playground.
      let spans: StoredSpan[] = LIST_SPANS
      const name = p.name as string | undefined
      const worker = p.service_name as string | undefined
      const status = p.status as string | undefined
      if (name) {
        const q = name.toLowerCase()
        spans = spans.filter((s) => s.name.toLowerCase().includes(q))
      }
      if (worker) spans = spans.filter((s) => s.service_name === worker)
      if (status) {
        spans = spans.filter(
          (s) =>
            (s.status.toLowerCase() === 'error' ? 'error' : 'ok') === status,
        )
      }
      return {
        spans,
        total: spans.length,
        offset: (p.offset as number) ?? 0,
        limit: (p.limit as number) ?? 500,
      }
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
