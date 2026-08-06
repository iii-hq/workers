import { Copy } from 'lucide-react'
import { useMemo } from 'react'
import { FunctionTriggerCard } from '@/components/function-trigger/FunctionTriggerCard'
import { Badge } from '@/components/ui/Badge'
import { StatusDot } from '@/components/ui/StatusDot'
import { functionTriggerFromSpan } from '../lib/functionTriggerFromSpan'
import type { VisualizationSpan, WaterfallData } from '../lib/traceTransform'
import {
  formatDuration,
  getWorkerName,
  useCopyToClipboard,
} from '../lib/traceUtils'

interface SpanInfoTabProps {
  span: VisualizationSpan
  traceData: WaterfallData | null
}

function statusTone(
  status: VisualizationSpan['status'],
): 'accent' | 'alert' | 'ink' {
  if (status === 'ok') return 'accent'
  if (status === 'error') return 'alert'
  return 'ink'
}

export function SpanInfoTab({ span, traceData }: SpanInfoTabProps) {
  const { copiedKey: copiedField, copy: copyToClipboard } = useCopyToClipboard()

  const worker = getWorkerName(span)
  const tracePercent = traceData
    ? (span.duration_ms / traceData.total_duration_ms) * 100
    : 0

  const tone = statusTone(span.status)

  // All spans of the trace, for resolving a nested span's OWNING function
  // via its ancestor chain (`trigger <fn>` / `call <fn>` invocation spans).
  const spansById = useMemo(
    () => new Map((traceData?.spans ?? []).map((s) => [s.span_id, s] as const)),
    [traceData],
  )

  // Function-invocation spans get the same call card chat renders — the
  // adapter synthesizes its data shape from the span's attributes/events.
  const functionTrigger = useMemo(
    () => functionTriggerFromSpan(span, spansById),
    [span, spansById],
  )

  return (
    <div className="p-5 space-y-5">
      {/* Function trigger — read-only card (no approval handlers) */}
      {functionTrigger && (
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint mb-2.5">
            function trigger
          </div>
          <FunctionTriggerCard message={functionTrigger} defaultOpen />
        </div>
      )}

      {/* Timing */}
      <div>
        <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint mb-2.5">
          timing
        </div>
        <div className="rounded-md bg-surface">
          <div className="px-4 pt-4 pb-3">
            <div className="flex items-baseline justify-between mb-2">
              <span className="font-mono text-xl font-bold text-accent tabular-nums">
                {formatDuration(span.duration_ms)}
                {span.pending && '…'}
              </span>
              {span.pending ? (
                <Badge variant="accent">running</Badge>
              ) : (
                traceData && (
                  <span className="font-mono text-[11px] text-ink-faint tabular-nums">
                    {tracePercent.toFixed(1)}% of trace
                  </span>
                )
              )}
            </div>
            <div className="h-1.5 bg-panel overflow-hidden">
              <div
                className={
                  span.pending
                    ? 'h-full bg-accent animate-pulse'
                    : 'h-full bg-accent transition-all duration-300'
                }
                style={{
                  width: span.pending
                    ? '100%'
                    : `${Math.min(100, Math.max(2, tracePercent))}%`,
                }}
              />
            </div>
          </div>
          <div className="border-t border-rule px-4 py-2.5 flex items-center justify-between">
            <span className="font-mono text-[11px] text-ink-faint lowercase">
              position in trace
            </span>
            <span className="font-mono text-[11px] text-ink tabular-nums">
              {span.start_percent.toFixed(1)}% →{' '}
              {Math.min(100, span.start_percent + span.width_percent).toFixed(
                1,
              )}
              %
            </span>
          </div>
        </div>
      </div>

      {/* Status */}
      <div>
        <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint mb-2.5">
          status
        </div>
        <div className="rounded-md bg-surface px-4 py-3">
          <div className="flex items-center gap-2.5">
            <StatusDot tone={tone} />
            <span className="font-mono text-[13px] font-semibold text-ink lowercase">
              {span.status}
            </span>
          </div>
        </div>
      </div>

      {/* Worker & Operation */}
      <div>
        <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint mb-2.5">
          worker & operation
        </div>
        <div className="rounded-md bg-surface divide-y divide-rule-2">
          <div className="px-4 py-2.5 flex items-center justify-between">
            <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
              worker
            </span>
            <span className="font-mono text-[13px] text-ink lowercase">
              {worker}
            </span>
          </div>
          <div className="px-4 py-2.5 flex items-center justify-between gap-4">
            <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint flex-shrink-0">
              operation
            </span>
            <span
              className="font-mono text-[13px] text-ink truncate lowercase"
              title={span.name}
            >
              {span.name}
            </span>
          </div>
          {span.depth === 0 && (
            <div className="px-4 py-2.5 flex items-center justify-between">
              <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
                type
              </span>
              <Badge variant="accent">root span</Badge>
            </div>
          )}
        </div>
      </div>

      {/* Identifiers */}
      <div>
        <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint mb-2.5">
          identifiers
        </div>
        <div className="rounded-md bg-surface divide-y divide-rule-2">
          {[
            { label: 'trace id', value: span.trace_id, field: 'traceId' },
            { label: 'span id', value: span.span_id, field: 'spanId' },
            ...(span.parent_span_id
              ? [
                  {
                    label: 'parent span',
                    value: span.parent_span_id,
                    field: 'parentId',
                  },
                ]
              : []),
          ].map(({ label, value, field }) => (
            <button
              key={field}
              type="button"
              onClick={() => copyToClipboard(field, value)}
              className="w-full px-4 py-2.5 flex items-center justify-between hover:bg-surface-hover transition-colors group text-left"
            >
              <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint flex-shrink-0">
                {label}
              </span>
              <div className="flex items-center gap-2 min-w-0 ml-4">
                <span
                  className="font-mono text-[11px] text-ink truncate tabular-nums"
                  title={value}
                >
                  {value}
                </span>
                {copiedField === field ? (
                  <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-accent flex-shrink-0">
                    copied
                  </span>
                ) : (
                  <Copy className="w-3 h-3 text-ink-ghost opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0" />
                )}
              </div>
            </button>
          ))}
        </div>
      </div>

      {/* Hierarchy */}
      <div>
        <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint mb-2.5">
          hierarchy
        </div>
        <div className="rounded-md bg-surface divide-y divide-rule-2">
          <div className="px-4 py-2.5 flex items-center justify-between">
            <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
              depth
            </span>
            <span className="font-mono text-[13px] text-ink tabular-nums">
              {span.depth}
            </span>
          </div>
          {span.flags !== undefined && (
            <div className="px-4 py-2.5 flex items-center justify-between">
              <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
                flags
              </span>
              <span className="font-mono text-[13px] text-ink tabular-nums">
                0x{span.flags.toString(16)}
              </span>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
