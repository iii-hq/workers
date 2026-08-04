import { useQuery } from '@tanstack/react-query'
import { ChevronDown, ChevronRight, Copy, FileText } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Cell } from '@/components/ui/Cell'
import { Skeleton } from '@/components/ui/Skeleton'
import { cn } from '@/lib/utils'
import { fetchOtelLogs, type OtelLog } from '../api/otel-logs'
import { toMs, type VisualizationSpan } from '../lib/traceTransform'
import {
  formatRelative,
  formatTimestamp,
  useCopyToClipboard,
} from '../lib/traceUtils'

/**
 * No `redact` prop, deliberately: this tab's data comes from
 * `engine::logs::list` — real OTel LOG records, a signal entirely separate
 * from the span/trace data `SpanTagsTab`/`SpanLogsTab`/`SpanErrorsTab` read.
 * A runtime_id can only reach it if code-runner actually emits OTel log
 * records, which it does not: the crate never calls
 * `iii_helpers::observability::Logger` (the only API that emits a real OTel
 * LogRecord — zero call sites in `code-runner/src`), and its own
 * `main.rs` builds its `tracing_subscriber` from a plain `fmt()` layer with
 * no OTel bridge attached, so its `tracing::info!`/`warn!`/`error!` calls
 * go to that process's own stdout only, never to the engine's log store.
 * (No code-runner log line carries a runtime_id today either — the closest,
 * `manager.rs`'s runtime-created line, logs the namespace and language.)
 * If the worker ever adopts `Logger` or a tracing→OTel bridge, revisit
 * this — see `SpanPanel.redaction-coverage.test.ts`, which enforces that
 * every tab has either a `redact` wiring or a written reason like this one.
 */
interface SpanOtelLogsTabProps {
  span: VisualizationSpan
}

type SeverityTone = 'error' | 'warn' | 'info' | 'debug'

interface SeverityStyle {
  label: string
  text: string
  rail: string
  cardBg: string
}

const SEVERITY_STYLES: Record<SeverityTone, SeverityStyle> = {
  error: {
    label: 'ERROR',
    text: 'text-alert',
    rail: 'border-l-2 border-l-alert',
    cardBg: 'bg-alert/5',
  },
  warn: {
    label: 'WARN',
    text: 'text-warn',
    rail: 'border-l-2 border-l-warn',
    cardBg: 'bg-warn/5',
  },
  info: {
    label: 'INFO',
    text: 'text-ink',
    rail: 'border-l-2 border-l-ink',
    cardBg: 'bg-bg',
  },
  debug: {
    label: 'DEBUG',
    text: 'text-ink-faint',
    rail: 'border-l-2 border-l-rule',
    cardBg: 'bg-bg',
  },
}

function getSeverity(severityText: string): SeverityStyle {
  const level = severityText?.toUpperCase() || ''
  if (level.startsWith('ERROR') || level.startsWith('FATAL'))
    return { ...SEVERITY_STYLES.error, label: level || 'ERROR' }
  if (level.startsWith('WARN'))
    return { ...SEVERITY_STYLES.warn, label: level || 'WARN' }
  if (level.startsWith('INFO'))
    return { ...SEVERITY_STYLES.info, label: level || 'INFO' }
  return { ...SEVERITY_STYLES.debug, label: level || 'DEBUG' }
}

// IDs already shown in the span header — don't duplicate.
const HIDDEN_ATTRS = new Set(['span_id', 'trace_id'])

function isJsonString(str: string): boolean {
  if (typeof str !== 'string') return false
  const trimmed = str.trim()
  return (
    (trimmed.startsWith('{') && trimmed.endsWith('}')) ||
    (trimmed.startsWith('[') && trimmed.endsWith(']'))
  )
}

function tryParseJson(value: unknown): {
  isJson: boolean
  parsed: unknown
  raw: string
} {
  const raw = typeof value === 'object' ? JSON.stringify(value) : String(value)
  if (typeof value === 'object' && value !== null) {
    return { isJson: true, parsed: value, raw }
  }
  if (typeof value === 'string' && isJsonString(value)) {
    try {
      return { isJson: true, parsed: JSON.parse(value), raw }
    } catch {
      /* not valid JSON */
    }
  }
  return { isJson: false, parsed: value, raw }
}

function JsonValue({ value }: { value: unknown }) {
  const [expanded, setExpanded] = useState(false)
  const { isJson, parsed, raw } = tryParseJson(value)

  if (!isJson) {
    return (
      <span className="font-mono text-[11px] text-ink break-all">{raw}</span>
    )
  }

  const pretty = JSON.stringify(parsed, null, 2)
  const lineCount = pretty.split('\n').length

  if (lineCount <= 2) {
    return (
      <span className="font-mono text-[11px] text-ink break-all">{raw}</span>
    )
  }

  return (
    <div className="flex-1 min-w-0">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-1 font-mono text-[11px] text-ink-faint hover:text-ink transition-colors"
      >
        {expanded ? (
          <ChevronDown className="w-3 h-3" />
        ) : (
          <ChevronRight className="w-3 h-3" />
        )}
        <span className="lowercase">
          {expanded ? 'collapse' : `${lineCount} lines`}
        </span>
      </button>
      {expanded ? (
        <pre className="mt-1.5 px-3 py-2 border border-rule bg-bg font-mono text-[12.5px] leading-[1.55] text-ink overflow-x-auto whitespace-pre-wrap break-all">
          {pretty}
        </pre>
      ) : (
        <div className="mt-1 font-mono text-[11px] text-ink-faint truncate">
          {raw}
        </div>
      )}
    </div>
  )
}

function CopyableValue({ label, value }: { label: string; value: string }) {
  const { copiedKey, copy } = useCopyToClipboard()
  const isCopied = copiedKey === label

  return (
    <button
      type="button"
      onClick={() => copy(label, value)}
      className="flex items-center gap-1 font-mono text-[11px] text-ink-faint hover:text-ink transition-colors group"
    >
      <span className="truncate">{value}</span>
      {isCopied ? (
        <span className="text-accent text-[10px] uppercase tracking-[0.06em] flex-shrink-0">
          copied
        </span>
      ) : (
        <Copy className="w-2.5 h-2.5 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0" />
      )}
    </button>
  )
}

interface LogCardProps {
  log: OtelLog
  index: number
  firstLogMs: number
}

function LogCard({ log, index, firstLogMs }: LogCardProps) {
  const logMs = toMs(log.timestamp_unix_nano)
  const offsetMs = logMs - firstLogMs
  const severity = getSeverity(log.severity_text)

  const dataAttrs: Array<[string, unknown]> = []
  const metaAttrs: Array<[string, unknown]> = []

  if (log.attributes) {
    for (const [key, value] of Object.entries(log.attributes)) {
      if (HIDDEN_ATTRS.has(key)) continue
      const { isJson } = tryParseJson(value)
      if (isJson || (typeof value === 'string' && value.length > 80)) {
        dataAttrs.push([key, value])
      } else {
        metaAttrs.push([key, value])
      }
    }
  }

  return (
    <div
      className={cn(
        'border border-rule overflow-hidden',
        severity.rail,
        severity.cardBg,
      )}
    >
      <div className="px-3.5 py-3">
        <div className="flex items-start justify-between gap-2 mb-1.5">
          <p
            className={cn(
              'font-mono text-[13px] font-medium leading-snug',
              severity.text,
            )}
          >
            {log.body}
          </p>
          <span
            className={cn(
              'font-mono text-[10px] font-medium uppercase tracking-[0.06em] flex-shrink-0',
              severity.text,
            )}
          >
            {severity.label}
          </span>
        </div>

        <div className="flex items-center gap-2 font-mono text-[10px] text-ink-faint mb-0.5 tabular-nums">
          <span className="text-ink-faint">{formatTimestamp(logMs)}</span>
          {index > 0 && (
            <span className="text-ink-ghost">{formatRelative(offsetMs)}</span>
          )}
          {log.service_name && (
            <>
              <span aria-hidden className="text-ink-ghost">
                ·
              </span>
              <span className="px-1 py-0.5 border border-rule bg-bg text-ink-faint lowercase">
                {log.service_name}
              </span>
            </>
          )}
        </div>

        {metaAttrs.length > 0 && (
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1 mt-2.5 pt-2.5 border-t border-rule-2">
            {metaAttrs.map(([key, value]) => (
              <div key={key} className="flex items-center gap-1.5 text-[10px]">
                <span className="text-ink-ghost font-mono lowercase">
                  {key}
                </span>
                <CopyableValue label={key} value={String(value)} />
              </div>
            ))}
          </div>
        )}

        {dataAttrs.length > 0 && (
          <div className="mt-2.5 pt-2.5 border-t border-rule-2 flex flex-col gap-2">
            {dataAttrs.map(([key, value]) => (
              <div key={key}>
                <span className="font-mono text-[10px] text-ink-ghost lowercase">
                  {key}
                </span>
                <div className="mt-0.5">
                  <JsonValue value={value} />
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}

export function SpanOtelLogsTab({ span }: SpanOtelLogsTabProps) {
  const { data, isLoading } = useQuery({
    queryKey: ['span-otel-logs', span.trace_id, span.span_id],
    queryFn: () =>
      fetchOtelLogs({
        trace_id: span.trace_id ?? undefined,
        span_id: span.span_id,
      }),
    enabled: !!span.trace_id && !!span.span_id,
  })

  const logs: OtelLog[] = data?.logs ?? []

  const { errorCount, warnCount } = useMemo(() => {
    let errorCount = 0
    let warnCount = 0
    for (const l of logs) {
      const sev = l.severity_text?.toUpperCase() ?? ''
      if (sev.startsWith('ERROR') || sev.startsWith('FATAL')) errorCount++
      else if (sev.startsWith('WARN')) warnCount++
    }
    return { errorCount, warnCount }
  }, [logs])

  // Precomputed stable keys: same log timestamp can appear twice in fast
  // sequences (the engine clock is nanosecond-resolution but coarse-grained
  // bursts can collide). Disambiguate with a monotonic counter so React
  // keys stay stable regardless of array index.
  const logKeys = useMemo(() => {
    const seen = new Map<string, number>()
    return logs.map((log) => {
      const base = `${log.timestamp_unix_nano}-${log.trace_id ?? ''}-${log.span_id ?? ''}`
      const n = (seen.get(base) ?? 0) + 1
      seen.set(base, n)
      return `${base}-${n}`
    })
  }, [logs])

  if (isLoading) {
    return (
      <div className="p-4 flex flex-col gap-2">
        {(['lg-sk-0', 'lg-sk-1', 'lg-sk-2'] as const).map((sk) => (
          <Skeleton key={sk} className="h-16 w-full" />
        ))}
      </div>
    )
  }

  if (logs.length === 0) {
    return (
      <div className="p-4">
        <Cell title="no logs found">
          <div className="flex items-start gap-3">
            <FileText
              aria-hidden
              className="w-4 h-4 text-ink-faint shrink-0 mt-0.5"
            />
            <p className="font-mono text-[12px] text-ink-faint lowercase">
              no log entries correlated with this span. logs appear once the
              engine ships otel log records carrying this trace_id and span_id.
            </p>
          </div>
        </Cell>
      </div>
    )
  }

  const firstLogMs = toMs(logs[0].timestamp_unix_nano)

  return (
    <div className="p-4 flex flex-col gap-2.5">
      <div className="flex items-center justify-between px-1">
        <span className="font-mono text-[11px] text-ink-faint tabular-nums lowercase">
          {logs.length} log{logs.length !== 1 ? 's' : ''} correlated
        </span>
        <div className="flex items-center gap-3 font-mono text-[10px] tabular-nums">
          {errorCount > 0 && (
            <span className="flex items-center gap-1 text-alert uppercase tracking-[0.06em]">
              <span aria-hidden className="w-1.5 h-1.5 bg-alert" />
              {errorCount} errors
            </span>
          )}
          {warnCount > 0 && (
            <span className="flex items-center gap-1 text-warn uppercase tracking-[0.06em]">
              <span aria-hidden className="w-1.5 h-1.5 bg-warn" />
              {warnCount} warnings
            </span>
          )}
        </div>
      </div>

      {logs.map((log, index) => (
        <LogCard
          key={logKeys[index]}
          log={log}
          index={index}
          firstLogMs={firstLogMs}
        />
      ))}
    </div>
  )
}
