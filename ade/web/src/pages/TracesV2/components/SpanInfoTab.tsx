import { Check, Copy } from 'lucide-react'
import { type ReactNode, useMemo } from 'react'
import {
  rawRedactor,
  useFunctionTriggerRenderers,
} from '@/components/function-trigger/renderer-registry'
import {
  FactMeter,
  factLabelClassName,
  factMonoValueClassName,
  factValueClassName,
} from '@/components/ui/FactCard'
import { cn } from '@/lib/utils'
import {
  functionTriggerFromSpan,
  spanFunctionId,
} from '../lib/functionTriggerFromSpan'
import { isLlmCall, llmCallFromSpan } from '../lib/llmCall'
import { getSpanKindIndicator } from '../lib/spanLabel'
import { getWorkerColor, SPAN_STATUS_COLORS } from '../lib/traceColors'
import type { VisualizationSpan, WaterfallData } from '../lib/traceTransform'
import {
  formatDuration,
  getWorkerName,
  useCopyToClipboard,
} from '../lib/traceUtils'
import { FunctionCallView } from './call/FunctionCallView'
import { LlmCallView } from './llm/LlmCallView'

interface SpanInfoTabProps {
  span: VisualizationSpan
  traceData: WaterfallData | null
}

/** A labelled group of rows: the quiet section title over a bordered list. */
function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-1.5">
      <h3 className="text-xs font-semibold text-foreground">{title}</h3>
      <div className="divide-y divide-border rounded-lg border border-border">
        {children}
      </div>
    </section>
  )
}

function Row({
  label,
  value,
  mono = false,
  title,
  onCopy,
  copied,
}: {
  label: string
  value: ReactNode
  mono?: boolean
  title?: string
  onCopy?: () => void
  copied?: boolean
}) {
  const body = (
    <>
      <span className={cn(factLabelClassName, 'w-24 shrink-0')}>{label}</span>
      <span
        className={cn(
          'min-w-0 flex-1 truncate text-right',
          mono ? factMonoValueClassName : factValueClassName,
        )}
        title={title}
      >
        {value}
      </span>
    </>
  )
  if (!onCopy) {
    return <div className="flex h-8 items-center gap-3 px-3">{body}</div>
  }
  return (
    <button
      type="button"
      onClick={onCopy}
      className="group/row flex h-8 w-full items-center gap-3 px-3 text-left transition-colors hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50"
      aria-label={`copy ${label}`}
    >
      {body}
      {copied ? (
        <Check className="size-4 shrink-0 text-ok" aria-hidden />
      ) : (
        <Copy
          className="size-4 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover/row:opacity-100 group-focus-visible/row:opacity-100"
          aria-hidden
        />
      )}
    </button>
  )
}

export function SpanInfoTab({ span, traceData }: SpanInfoTabProps) {
  const { copiedKey: copiedField, copy: copyToClipboard } = useCopyToClipboard()

  const worker = getWorkerName(span)
  const tracePercent = traceData
    ? (span.duration_ms / traceData.total_duration_ms) * 100
    : 0
  /* The bar's own colour on the meter: the worker hue, alert on error. */
  const accent =
    span.status === 'error' ? SPAN_STATUS_COLORS.error : getWorkerColor(worker)
  const kind = getSpanKindIndicator(span.kind)

  // All spans of the trace, for resolving a nested span's OWNING function
  // via its ancestor chain (`trigger <fn>` / `call <fn>` invocation spans).
  const spansById = useMemo(
    () => new Map((traceData?.spans ?? []).map((s) => [s.span_id, s] as const)),
    [traceData],
  )

  // Function-invocation spans render a call view built from the same
  // `FunctionTriggerMessage` the chat's FunctionTriggerCard consumes — the
  // adapter synthesizes it from the span's attributes/events — and apply
  // the SAME redactor the card would (rawRedactor, keyed on the function
  // id) before anything renders or reaches the clipboard. Model calls get
  // the conversation view, every other call the request/response view.
  const functionTrigger = useMemo(
    () => functionTriggerFromSpan(span, spansById),
    [span, spansById],
  )
  const renderers = useFunctionTriggerRenderers()
  const functionId = useMemo(
    () => spanFunctionId(span, spansById),
    [span, spansById],
  )
  const payloads = useMemo(() => {
    const redact = functionId ? rawRedactor(renderers, functionId) : undefined
    return {
      input: redact ? redact(functionTrigger?.input) : functionTrigger?.input,
      output: redact
        ? redact(functionTrigger?.output)
        : functionTrigger?.output,
    }
  }, [functionId, renderers, functionTrigger])
  const llm = useMemo(() => {
    const { input, output } = payloads
    if (!isLlmCall(functionId, span.attributes, input)) return null
    return {
      call: llmCallFromSpan(span.attributes, input, output),
      error:
        span.status === 'error' &&
        output &&
        typeof output === 'object' &&
        'error' in output
          ? output
          : undefined,
    }
  }, [functionId, payloads, span.attributes, span.status])

  const ids = [
    { label: 'trace', value: span.trace_id, field: 'traceId' },
    { label: 'span', value: span.span_id, field: 'spanId' },
    ...(span.parent_span_id
      ? [{ label: 'parent', value: span.parent_span_id, field: 'parentId' }]
      : []),
  ]

  return (
    <div className="@container flex flex-col gap-5 p-4">
      {llm ? (
        <section className="flex flex-col gap-2">
          <h3 className="text-xs font-semibold text-foreground">Model call</h3>
          <LlmCallView
            call={llm.call}
            functionId={functionId}
            durationMs={span.duration_ms}
            status={span.pending ? 'pending' : span.status}
            error={llm.error}
          />
        </section>
      ) : functionTrigger ? (
        <section className="flex flex-col gap-2">
          <h3 className="text-xs font-semibold text-foreground">
            Function call
          </h3>
          <FunctionCallView
            message={functionTrigger}
            rawInput={payloads.input}
            rawOutput={payloads.output}
            worker={worker}
            durationMs={span.duration_ms}
            status={span.pending ? 'pending' : span.status}
          />
        </section>
      ) : null}

      {/* Timing */}
      <section className="flex flex-col gap-1.5">
        <h3 className="text-xs font-semibold text-foreground">Timing</h3>
        <div className="rounded-lg border border-border px-3 py-2.5">
          <div className="flex items-baseline justify-between gap-3">
            <span className="font-mono text-lg font-semibold text-foreground tabular-nums">
              {formatDuration(span.duration_ms)}
              {span.pending ? '…' : null}
            </span>
            <span className="font-mono text-xs text-ink-faint tabular-nums">
              {span.pending
                ? 'running'
                : traceData
                  ? `${tracePercent.toFixed(1)}% of trace`
                  : null}
            </span>
          </div>
          <FactMeter
            value={tracePercent}
            accent={accent}
            label="share of the trace"
            pending={span.pending}
            className="mt-2 h-1.5"
          />
          <div className="mt-2 flex items-center justify-between font-mono text-xs text-ink-faint tabular-nums">
            <span>starts at {span.start_percent.toFixed(1)}%</span>
            <span>
              ends at{' '}
              {Math.min(100, span.start_percent + span.width_percent).toFixed(
                1,
              )}
              %
            </span>
          </div>
        </div>
      </section>

      <Group title="Details">
        <Row label="Worker" value={worker} mono />
        <Row label="Operation" value={span.name} mono title={span.name} />
        <Row
          label="Status"
          value={
            <span
              className={cn(
                'inline-flex items-center gap-1.5',
                span.status === 'error'
                  ? 'text-alert-strong'
                  : 'text-foreground',
              )}
            >
              <span
                aria-hidden
                className={cn(
                  'size-1.5 rounded-full',
                  span.status === 'error'
                    ? 'bg-alert'
                    : span.status === 'ok'
                      ? 'bg-ok'
                      : 'bg-muted-foreground',
                )}
              />
              {span.pending ? 'running' : span.status}
            </span>
          }
        />
        {kind ? <Row label="Kind" value={kind.label} /> : null}
        <Row
          label="Depth"
          value={span.depth === 0 ? 'root span' : String(span.depth)}
          mono
        />
        {span.flags !== undefined ? (
          <Row label="Flags" value={`0x${span.flags.toString(16)}`} mono />
        ) : null}
      </Group>

      <Group title="Identifiers">
        {ids.map(({ label, value, field }) => (
          <Row
            key={field}
            label={label}
            value={value}
            mono
            title={value}
            onCopy={() => copyToClipboard(field, value)}
            copied={copiedField === field}
          />
        ))}
      </Group>
    </div>
  )
}
