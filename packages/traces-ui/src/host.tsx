/**
 * What a host hands the traces UI: where the traces come from, where the
 * page keeps its settings, and the optional renderers for function calls.
 *
 * Every component that reads data or settings reads them from the nearest
 * `TracesProvider`; `TracesView` mounts one itself from its own props.
 */

import { createContext, type ReactNode, useContext, useMemo } from 'react'
import type { OtelLogsFilterParams, OtelLogsResponse } from './api/otel-logs'
import type {
  TraceSpansResponse,
  TracesFilterParams,
  TracesGroupByParams,
  TracesGroupByResponse,
  TracesResponse,
} from './api/traces'

/**
 * The read side of a trace store. `createEngineTraceSource` implements it
 * over an iii engine's `engine::traces::*` functions; any other backend
 * implements the same shapes.
 */
export interface TraceSource {
  /** One summary row per trace. */
  listTraces(options?: TracesFilterParams): Promise<TracesResponse>
  /** Full span records (events and links included). */
  traceSpans(options?: TracesFilterParams): Promise<TraceSpansResponse>
  /** Traces grouped by one attribute's value. */
  groupTraces(params: TracesGroupByParams): Promise<TracesGroupByResponse>
  /** Log records, filtered to a trace or a span. */
  logs(options?: OtelLogsFilterParams): Promise<OtelLogsResponse>
  /** Function ids whose spans start hidden. Absent: nothing starts hidden. */
  hiddenFunctionIds?(): Promise<ReadonlySet<string>>
  /**
   * Live updates: `onTraceIds` gets the ids of the traces whose spans
   * changed, and the page reads them again; `onResync` runs when the feed
   * may have missed changes (a reconnect), and the page reads everything
   * again. Returns the unsubscribe. Absent: the page shows what it read
   * and reads again when the tab becomes visible.
   */
  watch?(
    onTraceIds: (traceIds: string[]) => void,
    onResync?: () => void,
  ): () => void
}

export type TracesSettingsValue = Record<string, unknown>

/**
 * Where the page keeps its settings (saved views, the follow toggle, the
 * span filters): one JSON object, read and written whole. The page owns
 * the `traces` key of that object and leaves every other key as it found
 * it, so a host can share one document between several features.
 */
export interface TracesSettingsStore {
  /** The query key of the stored value in the host's QueryClient. */
  queryKey: readonly unknown[]
  /** The stored value, or `null` when there is nowhere to persist. */
  read(): Promise<TracesSettingsValue | null>
  /**
   * Queue a read-modify-write and return the value it will write. The
   * store applies `transform` to the freshest stored value when the write
   * runs and publishes the result under `queryKey`.
   */
  update(
    transform: (value: TracesSettingsValue) => TracesSettingsValue,
    fallback?: TracesSettingsValue,
  ): TracesSettingsValue
  /** Resolves once every queued update has been written (or failed). */
  settled(): Promise<void>
}

/** A function invocation, as a span describes it. */
export interface FunctionCallMessage {
  id: string
  role: 'function-call'
  createdAt: number
  functionId: string
  description?: string
  input: unknown
  output?: unknown
  durationMs?: number
  running?: boolean
  /**
   * The function id came from an enclosing invocation, not this record —
   * an inner span of `execute <fn>`; the card header stays verb-less.
   */
  identityInherited?: boolean
}

/**
 * A host renderer for one family of function calls: the span panel shows
 * its card instead of the raw request/response.
 */
export interface FunctionCallRenderer {
  id: string
  /** The page calls `tryRender*` only for function ids claimed here. */
  isMatch(functionId: string): boolean
  tryRender(message: FunctionCallMessage): ReactNode | null
  /** The tab title of the rendered card (default `Rendered`). */
  primaryTabLabel?: string
  /**
   * Redact the raw request/response before the page shows or copies it.
   * Pure and total: no mutation, no I/O, no throw for any shape.
   */
  redactRaw?(value: unknown): unknown
}

export interface TracesHost {
  source: TraceSource
  /** Absent: settings live in memory and saved views are hidden. */
  settings?: TracesSettingsStore
  functionCallRenderers?: readonly FunctionCallRenderer[]
}

const TracesHostContext = createContext<TracesHost | null>(null)

const NO_RENDERERS: readonly FunctionCallRenderer[] = []

export function TracesProvider({
  source,
  settings,
  functionCallRenderers,
  children,
}: TracesHost & { children: ReactNode }) {
  const value = useMemo(
    () => ({ source, settings, functionCallRenderers }),
    [source, settings, functionCallRenderers],
  )
  return (
    <TracesHostContext.Provider value={value}>
      {children}
    </TracesHostContext.Provider>
  )
}

function useTracesHost(): TracesHost {
  const host = useContext(TracesHostContext)
  if (!host) {
    throw new Error('traces components need a <TracesProvider> above them')
  }
  return host
}

export function useTraceSource(): TraceSource {
  return useTracesHost().source
}

export function useTracesSettings(): TracesSettingsStore | undefined {
  return useTracesHost().settings
}

export function useFunctionCallRenderers(): readonly FunctionCallRenderer[] {
  return useContext(TracesHostContext)?.functionCallRenderers ?? NO_RENDERERS
}

/** The first renderer that renders `pick`, with the node it rendered. */
export function firstRendered(
  renderers: readonly FunctionCallRenderer[],
  pick: (renderer: FunctionCallRenderer) => ReactNode | null,
): { renderer: FunctionCallRenderer; node: ReactNode } | null {
  for (const renderer of renderers) {
    const node = pick(renderer)
    if (node != null) return { renderer, node }
  }
  return null
}

/**
 * The redactor of the first renderer that declares `redactRaw` and claims
 * `functionId`; `undefined` when none does.
 */
export function rawRedactor(
  renderers: readonly FunctionCallRenderer[],
  functionId: string,
): ((value: unknown) => unknown) | undefined {
  for (const renderer of renderers) {
    if (renderer.redactRaw && renderer.isMatch(functionId)) {
      return (value) => renderer.redactRaw?.(value)
    }
  }
  return undefined
}
