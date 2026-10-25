import { useQuery } from '@tanstack/react-query'
import { ArrowUp, Check, Copy, X } from 'lucide-react'
import { useEffect, useMemo } from 'react'
import { useFunctionTriggerRenderers } from '@/components/function-trigger/renderer-registry'
import { Button } from '@/components/shadcn/button'
import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@/components/shadcn/tabs'
import { cn } from '@/lib/utils'
import { fetchOtelLogs } from '../api/otel-logs'
import { spanRawRedactor } from '../lib/functionTriggerFromSpan'
import { formatSpanLabel } from '../lib/spanLabel'
import { getWorkerColor } from '../lib/traceColors'
import type { VisualizationSpan, WaterfallData } from '../lib/traceTransform'
import {
  formatDuration,
  getWorkerName,
  useCopyToClipboard,
} from '../lib/traceUtils'
import { SpanBaggageTab } from './SpanBaggageTab'
import { SpanErrorsTab } from './SpanErrorsTab'
import { SpanInfoTab } from './SpanInfoTab'
import { SpanLinksTab } from './SpanLinksTab'
import { SpanLogsTab } from './SpanLogsTab'
import { SpanOtelLogsTab } from './SpanOtelLogsTab'
import { SpanTagsTab } from './SpanTagsTab'

interface SpanPanelProps {
  span: VisualizationSpan | null
  traceData: WaterfallData | null
  onClose: () => void
  onNavigateToSpan: (span: VisualizationSpan) => void
  onNavigateToTrace?: (traceId: string) => void
}

/** A small count beside a tab label. */
function TabCount({ count }: { count: number }) {
  if (count <= 0) return null
  return (
    <span className="ml-1 inline-flex h-4 min-w-4 items-center justify-center rounded-md bg-muted px-1 font-mono text-[11px] font-medium text-muted-foreground tabular-nums">
      {count}
    </span>
  )
}

/** One fact in the header's meta row. */
function Meta({
  label,
  value,
  tone,
}: {
  label: string
  value: string
  tone?: 'alert' | 'ok'
}) {
  return (
    <span className="inline-flex items-baseline gap-1 whitespace-nowrap">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span
        className={cn(
          'font-mono text-xs text-foreground tabular-nums',
          tone === 'alert' && 'text-alert-strong',
          tone === 'ok' && 'text-ok',
        )}
      >
        {value}
      </span>
    </span>
  )
}

export function SpanPanel({
  span,
  traceData,
  onClose,
  onNavigateToSpan,
  onNavigateToTrace,
}: SpanPanelProps) {
  const { copiedKey: copiedField, copy: copyToClipboard } = useCopyToClipboard()

  useEffect(() => {
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('keydown', handleEscape)
    return () => document.removeEventListener('keydown', handleEscape)
  }, [onClose])

  const traceContext = useMemo(() => {
    if (!span || !traceData) return null
    const parentSpan = traceData.spans.find(
      (s) => s.span_id === span.parent_span_id,
    )
    const childSpans = traceData.spans.filter(
      (s) => s.parent_span_id === span.span_id,
    )
    const childDuration = childSpans.reduce(
      (sum, child) => sum + child.duration_ms,
      0,
    )
    // Note: self-time is approximate when child spans overlap (parallel execution)
    const selfTime = Math.max(0, span.duration_ms - childDuration)
    return { parentSpan, childSpans, selfTime, childDuration }
  }, [span, traceData])

  // All spans of the trace, for resolving a nested span's owning function —
  // same ancestor-chain lookup `SpanInfoTab` uses to build the info card.
  const spansById = useMemo(
    () => new Map((traceData?.spans ?? []).map((s) => [s.span_id, s] as const)),
    [traceData],
  )
  const renderers = useFunctionTriggerRenderers()
  // Every tab below that reads `span.attributes`/`span.events` renders the
  // SAME data the info tab's card is built from (functionTriggerFromSpan.ts)
  // — the identical redactor has to cover them (tags, logs, errors), or the
  // runtime_id the info card hides is one tab-click away. The remaining tabs
  // (otel-logs, baggage, links) are exempt for a written reason at their own
  // definition — see `SpanPanel.redaction-coverage.test.ts`, which fails
  // loudly if a tab is ever added here without one or the other.
  const redact = useMemo(
    () => (span ? spanRawRedactor(span, spansById, renderers) : undefined),
    [span, spansById, renderers],
  )

  const { data: logsData } = useQuery({
    queryKey: ['span-otel-logs', span?.trace_id, span?.span_id],
    queryFn: () =>
      fetchOtelLogs({
        trace_id: span?.trace_id ?? undefined,
        span_id: span?.span_id,
      }),
    enabled: !!span?.trace_id && !!span?.span_id,
  })
  const logCount = logsData?.logs.length ?? 0

  if (!span) return null

  const hasError = span.status === 'error'
  const attrCount = Object.keys(span.attributes || {}).length
  const eventCount = span.events?.length || 0
  const linkCount = span.links?.length || 0
  const worker = getWorkerName(span)
  const color = getWorkerColor(worker)
  const title = formatSpanLabel(span)
  const childCount = traceContext?.childSpans.length ?? 0

  return (
    <div
      className="flex h-full flex-col overflow-hidden bg-card font-inter"
      data-span-panel
      data-span-id={span.span_id}
      data-span-name={span.name}
    >
      {/* Header: identity, then the facts, then the way up. */}
      <div className="shrink-0 border-b border-border px-4 pt-3 pb-3">
        <div className="flex items-start gap-2">
          <span
            aria-hidden
            className="mt-1 size-2 shrink-0 rounded-full"
            style={{
              backgroundColor: hasError ? 'var(--color-trace-error)' : color,
            }}
          />
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <h2
              className="truncate font-mono text-[13px] font-semibold leading-5 text-foreground"
              title={span.name}
            >
              {title}
            </h2>
            <div className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
              <span className="truncate">{worker}</span>
              <span aria-hidden>·</span>
              <button
                type="button"
                onClick={() => copyToClipboard('spanId', span.span_id)}
                className="group/id inline-flex shrink-0 items-center gap-1 rounded-sm font-mono tabular-nums transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
                title="copy span id"
                aria-label="copy span id"
              >
                {span.span_id.slice(0, 12)}
                {copiedField === 'spanId' ? (
                  <Check className="size-4 text-ok" aria-hidden />
                ) : (
                  <Copy
                    className="size-4 opacity-0 transition-opacity group-hover/id:opacity-100 group-focus-visible/id:opacity-100"
                    aria-hidden
                  />
                )}
              </button>
            </div>
          </div>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            onClick={onClose}
            aria-label="close panel (esc)"
            title="Close (Esc)"
            className="-mt-0.5 -mr-1.5"
          >
            <X />
          </Button>
        </div>

        <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1">
          <Meta
            label="status"
            value={span.pending ? 'running' : span.status}
            tone={hasError ? 'alert' : span.status === 'ok' ? 'ok' : undefined}
          />
          <Meta label="duration" value={formatDuration(span.duration_ms)} />
          {traceContext && childCount > 0 ? (
            <Meta label="self" value={formatDuration(traceContext.selfTime)} />
          ) : null}
          <Meta label="depth" value={String(span.depth)} />
          {childCount > 0 ? (
            <Meta label="children" value={String(childCount)} />
          ) : null}
        </div>

        {traceContext?.parentSpan ? (
          <button
            type="button"
            onClick={() =>
              traceContext.parentSpan &&
              onNavigateToSpan(traceContext.parentSpan)
            }
            className="group/parent mt-2.5 flex h-8 w-full items-center gap-1.5 rounded-md border border-border px-2 text-left text-xs transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
            title="open the parent span"
          >
            <ArrowUp
              className="size-4 shrink-0 text-muted-foreground transition-colors group-hover/parent:text-foreground"
              aria-hidden
            />
            <span className="shrink-0 text-muted-foreground">parent</span>
            <span className="min-w-0 flex-1 truncate font-mono text-foreground">
              {formatSpanLabel(traceContext.parentSpan)}
            </span>
            <span className="shrink-0 font-mono text-muted-foreground tabular-nums">
              {formatDuration(traceContext.parentSpan.duration_ms)}
            </span>
          </button>
        ) : null}
      </div>

      {/* Tabbed content.
        Intentionally not keyed on span_id so the active tab persists when the
        user navigates between spans. */}
      <div className="min-h-0 flex-1 overflow-hidden">
        <Tabs
          defaultValue={hasError ? 'errors' : 'info'}
          className="flex h-full flex-col gap-0"
        >
          <div className="shrink-0 overflow-x-auto border-b border-border px-4 no-scrollbar">
            <TabsList
              variant="line"
              className="h-9 w-max min-w-full justify-start gap-3 p-0"
            >
              <TabsTrigger value="info" className="flex-none px-0 text-xs">
                Info
              </TabsTrigger>
              <TabsTrigger value="tags" className="flex-none px-0 text-xs">
                Attributes
                <TabCount count={attrCount} />
              </TabsTrigger>
              <TabsTrigger value="logs" className="flex-none px-0 text-xs">
                Events
                <TabCount count={eventCount} />
              </TabsTrigger>
              <TabsTrigger value="errors" className="flex-none px-0 text-xs">
                Errors
                {hasError ? (
                  <>
                    <span
                      aria-hidden
                      className="ml-1 inline-block size-1.5 rounded-full bg-alert"
                    />
                    <span className="sr-only">this span failed</span>
                  </>
                ) : null}
              </TabsTrigger>
              <TabsTrigger value="otel-logs" className="flex-none px-0 text-xs">
                Logs
                <TabCount count={logCount} />
              </TabsTrigger>
              <TabsTrigger value="baggage" className="flex-none px-0 text-xs">
                Context
              </TabsTrigger>
              {linkCount > 0 ? (
                <TabsTrigger value="links" className="flex-none px-0 text-xs">
                  Links
                  <TabCount count={linkCount} />
                </TabsTrigger>
              ) : null}
            </TabsList>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
            <TabsContent value="info" className="mt-0">
              <SpanInfoTab span={span} traceData={traceData} />
            </TabsContent>
            <TabsContent value="tags" className="mt-0">
              <SpanTagsTab span={span} redact={redact} />
            </TabsContent>
            <TabsContent value="logs" className="mt-0">
              <SpanLogsTab span={span} redact={redact} />
            </TabsContent>
            <TabsContent value="errors" className="mt-0">
              <SpanErrorsTab span={span} redact={redact} />
            </TabsContent>
            <TabsContent value="otel-logs" className="mt-0">
              <SpanOtelLogsTab span={span} />
            </TabsContent>
            <TabsContent value="baggage" className="mt-0">
              <SpanBaggageTab span={span} />
            </TabsContent>
            {linkCount > 0 && (
              <TabsContent value="links" className="mt-0">
                <SpanLinksTab
                  span={span}
                  onNavigateToTrace={onNavigateToTrace}
                />
              </TabsContent>
            )}
          </div>
        </Tabs>
      </div>
    </div>
  )
}
