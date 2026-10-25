import { useQuery } from '@tanstack/react-query'
import { ChevronDown, ChevronRight, Copy, FileText } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Cell } from '@/components/ui/Cell'
import { Skeleton } from '@/components/ui/Skeleton'
import { JsonHighlight } from '@/lib/syntax'
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
 * A runtime_id can only reach it if sandbox-code-runner actually emits OTel
 * log records, which it does not: the crate never calls
 * `iii_helpers::observability::Logger` (the only API that emits a real OTel
 * LogRecord — zero call sites in `sandbox-code-runner/src`), and its own
 * `main.rs` builds its `tracing_subscriber` from a plain `fmt()` layer with
 * no OTel bridge attached, so its `tracing::info!`/`warn!`/`error!` calls
 * go to that process's own stdout only, never to the engine's log store.
 * (No sandbox-code-runner log line carries a runtime_id today either — the
 * closest, `manager.rs`'s runtime-created line, logs the namespace and
 * language.)
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
    text: 'text-alert-strong',
    rail: 'border-alert/30',
    cardBg: 'bg-alert-muted',
  },
  warn: {
    label: 'WARN',
    text: 'text-warn-strong',
    rail: 'border-warn/30',
    cardBg: 'bg-warn-muted',
  },
  info: {
    label: 'INFO',
    text: 'text-foreground',
    rail: 'border-border',
    cardBg: 'bg-card',
  },
  debug: {
    label: 'DEBUG',
    text: 'text-muted-foreground',
    rail: 'border-border',
    cardBg: 'bg-card',
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
      <span className="font-mono text-[11px] break-all text-foreground">
        {raw}
      </span>
    )
  }

  const pretty = JSON.stringify(parsed, null, 2)
  const lineCount = pretty.split('\n').length

  if (lineCount <= 2) {
    return (
      <span className="font-mono text-[11px] break-all text-foreground">
        {raw}
      </span>
    )
  }

  return (
    <div className="min-w-0 flex-1">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        aria-expanded={expanded}
        className="flex items-center gap-1 font-mono text-[11px] text-muted-foreground transition-colors hover:text-foreground"
      >
        {expanded ? (
          <ChevronDown className="size-4" aria-hidden />
        ) : (
          <ChevronRight className="size-4" aria-hidden />
        )}
        <span>{expanded ? 'Collapse' : `${lineCount} lines`}</span>
      </button>
      {expanded ? (
        <div className="code-theme-editor mt-1.5 max-h-80 overflow-auto overscroll-contain rounded-md border border-border">
          <JsonHighlight code={pretty} wrap className="px-3 py-2" />
        </div>
      ) : (
        <div className="mt-1 truncate font-mono text-[11px] text-muted-foreground">
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
      className="group/val flex items-center gap-1 font-mono text-[11px] text-foreground transition-colors hover:text-foreground"
      aria-label={`copy ${label}`}
    >
      <span className="truncate">{value}</span>
      {isCopied ? (
        <span className="shrink-0 text-ok">copied</span>
      ) : (
        <Copy
          className="size-4 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover/val:opacity-100"
          aria-hidden
        />
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
        'overflow-hidden rounded-lg border',
        severity.rail,
        severity.cardBg,
      )}
    >
      <div className="px-3 py-2">
        <div className="mb-1 flex items-start justify-between gap-2">
          <p
            className={cn(
              'font-mono text-xs leading-snug break-words whitespace-pre-wrap',
              severity.text,
            )}
          >
            {log.body}
          </p>
          <span
            className={cn(
              'shrink-0 font-mono text-[11px] font-semibold tracking-wide',
              severity.text,
            )}
          >
            {severity.label}
          </span>
        </div>

        <div className="flex flex-wrap items-center gap-1.5 font-mono text-[11px] text-muted-foreground tabular-nums">
          <span>{formatTimestamp(logMs)}</span>
          {index > 0 && <span>{formatRelative(offsetMs)}</span>}
          {log.service_name && (
            <>
              <span aria-hidden>·</span>
              <span className="rounded-md bg-muted px-1 py-px">
                {log.service_name}
              </span>
            </>
          )}
        </div>

        {metaAttrs.length > 0 && (
          <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-border pt-2">
            {metaAttrs.map(([key, value]) => (
              <div key={key} className="flex items-center gap-1.5 text-[11px]">
                <span className="font-mono text-muted-foreground">{key}</span>
                <CopyableValue label={key} value={String(value)} />
              </div>
            ))}
          </div>
        )}

        {dataAttrs.length > 0 && (
          <div className="mt-2 flex flex-col gap-2 border-t border-border pt-2">
            {dataAttrs.map(([key, value]) => (
              <div key={key}>
                <span className="font-mono text-[11px] text-muted-foreground">
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
      <div className="flex flex-col gap-2 p-4" aria-busy="true">
        {(['lg-sk-0', 'lg-sk-1', 'lg-sk-2'] as const).map((sk) => (
          <Skeleton key={sk} className="h-14 w-full rounded-lg" />
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
              className="mt-0.5 size-4 shrink-0 text-muted-foreground"
            />
            <p className="font-inter text-[13px] text-muted-foreground">
              No log entries correlated with this span. Logs appear once the
              engine ships OTel log records carrying this trace_id and span_id.
            </p>
          </div>
        </Cell>
      </div>
    )
  }

  const firstLogMs = toMs(logs[0].timestamp_unix_nano)

  return (
    <div className="flex flex-col gap-2 p-4 font-inter">
      <div className="flex h-6 items-center justify-between">
        <span className="text-xs font-semibold text-foreground">
          Logs{' '}
          <span className="font-mono text-[11px] font-normal text-muted-foreground tabular-nums">
            {logs.length}
          </span>
        </span>
        <div className="flex items-center gap-3 font-mono text-[11px] tabular-nums">
          {errorCount > 0 && (
            <span className="flex items-center gap-1 text-alert-strong">
              <span aria-hidden className="size-1.5 rounded-full bg-alert" />
              {errorCount} errors
            </span>
          )}
          {warnCount > 0 && (
            <span className="flex items-center gap-1 text-warn-strong">
              <span aria-hidden className="size-1.5 rounded-full bg-warn" />
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
