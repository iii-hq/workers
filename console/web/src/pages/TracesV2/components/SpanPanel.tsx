import { useQuery } from '@tanstack/react-query'
import { ArrowUp, Clock, Copy, Layers, X, Zap } from 'lucide-react'
import { useEffect, useMemo } from 'react'
import { Button } from '@/components/ui/Button'
import { StatusDot } from '@/components/ui/StatusDot'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/Tabs'
import { cn } from '@/lib/utils'
import { fetchOtelLogs } from '../api/otel-logs'
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

function statusTone(
  status: VisualizationSpan['status'],
): 'accent' | 'alert' | 'ink' {
  if (status === 'ok') return 'accent'
  if (status === 'error') return 'alert'
  return 'ink'
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
  const tone = statusTone(span.status)

  return (
    <div
      className="h-full bg-panel-raised overflow-hidden flex flex-col"
      data-span-panel
      data-span-id={span.span_id}
      data-span-name={span.name}
    >
      {/* Header strip */}
      <div className="flex-shrink-0 border-b border-rule-2">
        {/* Row 1: worker badge + span name + close */}
        <div className="flex items-center gap-2 px-4 pt-3 pb-1.5">
          <span className="px-1.5 py-0.5 font-mono text-[11px] uppercase tracking-[0.06em] flex-shrink-0 rounded-xs bg-surface text-ink-faint lowercase">
            {worker}
          </span>
          <h2
            className="font-mono text-[13px] font-semibold text-ink leading-tight truncate flex-1 min-w-0 lowercase"
            title={span.name}
          >
            {span.name}
          </h2>
          <Button
            variant="icon"
            size="icon"
            onClick={onClose}
            aria-label="close panel (esc)"
            title="close (esc)"
          >
            <X className="w-3.5 h-3.5" />
          </Button>
        </div>

        {/* Row 2: span ID + quick stats */}
        <div className="flex items-center gap-2 px-4 pb-2.5 flex-wrap">
          <button
            type="button"
            onClick={() => copyToClipboard('spanId', span.span_id)}
            className="flex items-center gap-1 font-mono text-[11px] text-ink-faint hover:text-ink transition-colors group tabular-nums"
          >
            <span>{span.span_id.slice(0, 12)}</span>
            {copiedField === 'spanId' ? (
              <span className="text-accent text-[10px] uppercase tracking-[0.06em]">
                copied
              </span>
            ) : (
              <Copy className="w-2.5 h-2.5 opacity-0 group-hover:opacity-100 transition-opacity" />
            )}
          </button>

          <span aria-hidden className="w-px h-3 bg-rule-2" />

          <span className="flex items-center gap-1 px-2 py-0.5 rounded-xs bg-surface">
            <Clock className="w-2.5 h-2.5 text-accent" />
            <span className="font-mono text-[11px] font-semibold text-accent tabular-nums">
              {formatDuration(span.duration_ms)}
            </span>
          </span>

          {traceContext && traceContext.childSpans.length > 0 && (
            <span className="flex items-center gap-1 px-2 py-0.5 rounded-xs bg-surface">
              <Zap className="w-2.5 h-2.5 text-ink-faint" />
              <span className="font-mono text-[11px] text-ink-faint tabular-nums lowercase">
                self {formatDuration(traceContext.selfTime)}
              </span>
            </span>
          )}

          <span
            className={cn(
              'flex items-center gap-1 px-2 py-0.5 rounded-xs',
              span.status === 'error' ? 'bg-alert-muted' : 'bg-surface',
            )}
          >
            <StatusDot tone={tone} />
            <span
              className={cn(
                'font-mono text-[11px] uppercase tracking-[0.06em]',
                span.status === 'error' ? 'text-alert' : 'text-ink-faint',
              )}
            >
              {span.status}
            </span>
          </span>

          <span className="flex items-center gap-1 px-2 py-0.5 rounded-xs bg-surface">
            <Layers className="w-2.5 h-2.5 text-ink-faint" />
            <span className="font-mono text-[11px] text-ink-faint tabular-nums">
              d:{span.depth}
            </span>
          </span>

          {traceContext && traceContext.childSpans.length > 0 && (
            <span className="flex items-center gap-1 px-2 py-0.5 rounded-xs bg-surface">
              <span className="font-mono text-[11px] text-ink-faint tabular-nums lowercase">
                {traceContext.childSpans.length} child
                {traceContext.childSpans.length !== 1 ? 'ren' : ''}
              </span>
            </span>
          )}
        </div>

        {/* Parent span navigation */}
        {traceContext?.parentSpan && (
          <div className="px-4 pb-2.5">
            <button
              type="button"
              onClick={() =>
                traceContext.parentSpan &&
                onNavigateToSpan(traceContext.parentSpan)
              }
              className="flex items-center gap-1.5 w-full px-2.5 py-1.5 rounded-sm bg-surface hover:bg-surface-hover transition-colors group text-left"
            >
              <ArrowUp className="w-3 h-3 text-ink-faint group-hover:text-accent transition-colors flex-shrink-0" />
              <span className="font-mono text-[11px] text-ink-faint flex-shrink-0 lowercase">
                parent
              </span>
              <span className="font-mono text-[11px] text-ink truncate group-hover:text-ink transition-colors lowercase">
                {traceContext.parentSpan.name}
              </span>
              <span className="font-mono text-[10px] text-ink-ghost ml-auto flex-shrink-0 tabular-nums">
                {formatDuration(traceContext.parentSpan.duration_ms)}
              </span>
            </button>
          </div>
        )}
      </div>

      {/* Tabbed content.
        Intentionally not keyed on span_id so the active tab persists when the
        user navigates between spans. */}
      <div className="flex-1 overflow-hidden min-h-0">
        <Tabs
          defaultValue={hasError ? 'errors' : 'info'}
          className="h-full flex flex-col"
        >
          <TabsList className="px-4 pt-2 pb-0 bg-transparent">
            <TabsTrigger value="info">info</TabsTrigger>
            <TabsTrigger value="tags">
              attributes
              {attrCount > 0 && (
                <span className="ml-1 px-1 py-0.5 font-mono text-[10px] tabular-nums text-ink-faint rounded-xs bg-surface normal-case tracking-normal">
                  {attrCount}
                </span>
              )}
            </TabsTrigger>
            <TabsTrigger value="logs">
              events
              {eventCount > 0 && (
                <span className="ml-1 px-1 py-0.5 font-mono text-[10px] tabular-nums text-ink-faint rounded-xs bg-surface normal-case tracking-normal">
                  {eventCount}
                </span>
              )}
            </TabsTrigger>
            <TabsTrigger value="errors">
              errors
              {hasError && (
                <span className="ml-1 w-1.5 h-1.5 bg-alert rounded-full inline-block" />
              )}
            </TabsTrigger>
            <TabsTrigger value="otel-logs">
              logs
              {logCount > 0 && (
                <span className="ml-1 px-1 py-0.5 font-mono text-[10px] tabular-nums text-ink-faint rounded-xs bg-surface normal-case tracking-normal">
                  {logCount}
                </span>
              )}
            </TabsTrigger>
            <TabsTrigger value="baggage">context</TabsTrigger>
            {linkCount > 0 && (
              <TabsTrigger value="links">
                links
                <span className="ml-1 px-1 py-0.5 font-mono text-[10px] tabular-nums text-ink-faint rounded-xs bg-surface normal-case tracking-normal">
                  {linkCount}
                </span>
              </TabsTrigger>
            )}
          </TabsList>

          <div className="flex-1 overflow-y-auto min-h-0">
            <TabsContent value="info" className="mt-0">
              <SpanInfoTab span={span} traceData={traceData} />
            </TabsContent>
            <TabsContent value="tags" className="mt-0">
              <SpanTagsTab span={span} />
            </TabsContent>
            <TabsContent value="logs" className="mt-0">
              <SpanLogsTab span={span} />
            </TabsContent>
            <TabsContent value="errors" className="mt-0">
              <SpanErrorsTab span={span} />
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
