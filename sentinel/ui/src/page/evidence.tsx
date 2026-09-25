import {
  Button,
  Card,
  CardBody,
  CardHeader,
  Chip,
  CodeHighlight,
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
  EmptyState,
  Skeleton,
  StatusPanel,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import type { Host } from '@iii-dev/console-ui'
import { Check, ChevronDown, Copy } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { Client, EvidenceBundle, EvidenceSpan, OccurrenceSummary } from '../api'
import { Dot } from './marks'
import { ago, stamp } from './present.js'

interface Props {
  api: Client
  host: Host
  now: number
  occurrence: OccurrenceSummary
}

/** How much of one payload is worth reading inline. The worker caps what it
    stores; this caps what the page puts on screen at once. */
const VALUE_PREVIEW = 1200

/**
 * The frozen bundle. It is a snapshot by design — by the time somebody opens
 * a group the engine's ring has usually dropped the trace — so the view says
 * when it was captured rather than pretending to be live.
 */
export function EvidenceView({ api, now, occurrence }: Props) {
  const [bundle, setBundle] = useState<EvidenceBundle | null>(null)
  const [pruned, setPruned] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [attempt, setAttempt] = useState(0)

  // The previous bundle stays on screen while the next one loads: a new
  // occurrence arriving every few seconds must not blank what is being read.
  useEffect(() => {
    let live = true
    setError(null)
    api
      .evidence(occurrence.id)
      .then((response) => {
        if (!live) return
        setBundle(response.evidence ?? null)
        setPruned(response.pruned || !response.evidence)
      })
      .catch((cause) => live && setError(errorMessage(cause)))
    return () => {
      live = false
    }
  }, [api, occurrence.id, attempt])

  if (error) {
    return (
      <StatusPanel
        variant="alert"
        headline="Could not read the evidence"
        detail={error}
        action={
          <Button size="sm" onClick={() => setAttempt((previous) => previous + 1)}>
            Retry
          </Button>
        }
      />
    )
  }
  if (pruned) {
    return (
      <EmptyState
        compact
        title="The evidence was pruned"
        description="Retention kept the occurrence and let its bundle go. The first occurrence, the most recent ones and one per worker version still have theirs."
      />
    )
  }
  if (!bundle) return <Skeleton />

  const captured = `captured ${ago(bundle.captured_at_ms, now)} · ${bundle.settled ? 'settled' : 'still open'}`
  return (
    <div className="sentinel-ui-stack">
      {occurrence.source === 'log' ? (
        <LogRecord bundle={bundle} captured={captured} />
      ) : (
        <OriginSpan bundle={bundle} captured={captured} occurrence={occurrence} />
      )}
      {occurrence.source === 'log' ? null : <TracePath bundle={bundle} />}
      <Payloads bundle={bundle} />
      {bundle.logs.length > 0 && occurrence.source !== 'log' ? <TraceLogs bundle={bundle} /> : null}
    </div>
  )
}

function OriginSpan({
  bundle,
  captured,
  occurrence,
}: {
  bundle: EvidenceBundle
  captured: string
  occurrence: OccurrenceSummary
}) {
  const origin = bundle.spans.find((span) => span.span_id === bundle.origin_span_id)
  const exception = exceptionOf(origin)
  const type = exception['exception.type']
  const message = exception['exception.message'] ?? origin?.status_description
  const stack = exception['exception.stacktrace']
  const service = [origin?.service_name ?? bundle.worker.service_name, occurrence.worker_version ?? bundle.worker.version]
    .filter(Boolean)
    .join(' · ')

  return (
    <Card>
      <CardHeader>
        <span>Origin span</span>
        <span className="sentinel-ui-card-note sentinel-ui-mono">{captured}</span>
        <CopyTrace traceId={bundle.trace_id} />
      </CardHeader>
      <CardBody className="sentinel-ui-origin">
        <dl className="sentinel-ui-kv">
          <dt>span</dt>
          <dd>{origin?.name ?? '—'}</dd>
          <dt>function_id</dt>
          <dd>{origin?.function_id ?? origin?.attributes.function_id ?? '—'}</dd>
          <dt>service</dt>
          <dd>{service || '—'}</dd>
          <dt>status</dt>
          <dd>
            <span className="sentinel-ui-alert">error</span>
            {type ? ` · ${type}` : ''}
          </dd>
          <dt>duration</dt>
          <dd>{origin ? duration(origin) : '—'}</dd>
        </dl>
        <dl className="sentinel-ui-kv">
          <dt>trace_id</dt>
          <dd title={bundle.trace_id}>{short(bundle.trace_id)}</dd>
          <dt>session</dt>
          <dd>{occurrence.session_id ?? '—'}</dd>
          <dt>turn</dt>
          <dd>{occurrence.turn_id ?? '—'}</dd>
          <dt>iii.tag.kind</dt>
          <dd>{bundle.trace_tags['iii.tag.kind'] ?? origin?.attributes['iii.tag.kind'] ?? '—'}</dd>
          <dt>propagated through</dt>
          <dd>
            {bundle.propagated_through.length} {bundle.propagated_through.length === 1 ? 'span' : 'spans'}
          </dd>
        </dl>
        {type || message || stack ? (
          <pre className="sentinel-ui-exception">
            {type ? (
              <>
                <span className="sentinel-ui-exception-key">exception.type</span>
                <span className="sentinel-ui-alert">{type}</span>
              </>
            ) : null}
            {message ? (
              <>
                <span className="sentinel-ui-exception-key">exception.message</span>
                <span>{message}</span>
              </>
            ) : null}
            {stack ? (
              <>
                <span className="sentinel-ui-exception-key">exception.stacktrace</span>
                <span>{stack}</span>
              </>
            ) : null}
          </pre>
        ) : null}
      </CardBody>
    </Card>
  )
}

function TracePath({ bundle }: { bundle: EvidenceBundle }) {
  if (bundle.spans.length === 0) return null
  return (
    <Card>
      <CardHeader>
        <span>Trace path</span>
        <span className="sentinel-ui-card-note">
          error spans above the origin are propagation, not separate issues
        </span>
      </CardHeader>
      <CardBody>
        <ol className="sentinel-ui-spans">
          {bundle.spans.map((span) => {
            const origin = span.span_id === bundle.origin_span_id
            const carried = bundle.propagated_through.includes(span.span_id)
            return (
              <li
                key={span.span_id}
                className="sentinel-ui-span"
                style={{ paddingLeft: `${8 + Math.min(span.depth, 8) * 16}px` }}
                data-origin={origin ? 'true' : undefined}
              >
                <Dot tone={isError(span) ? 'alert' : 'ghost'} />
                <span className="sentinel-ui-span-name">{span.name}</span>
                <span className="sentinel-ui-span-service">{span.service_name}</span>
                {carried ? <Chip tone="warning">propagated</Chip> : null}
                {origin ? <Chip tone="danger">origin</Chip> : null}
                {span.end_time_unix_nano === 0 ? <Chip tone="warning">still open</Chip> : null}
                <span className="sentinel-ui-span-duration">{duration(span)}</span>
              </li>
            )
          })}
        </ol>
        {bundle.truncated.spans > 0 ? (
          <p className="sentinel-ui-card-foot">
            {bundle.truncated.spans} spans furthest from the origin were dropped to fit the evidence budget.
          </p>
        ) : null}
      </CardBody>
    </Card>
  )
}

/** The SDK's own request and response events: frequently where the answer
    is and always where the noise is, so one click away. */
function Payloads({ bundle }: { bundle: EvidenceBundle }) {
  const origin = bundle.spans.find((span) => span.span_id === bundle.origin_span_id)
  const payloads = (origin?.events ?? [])
    .filter((event) => event.name !== 'exception')
    .flatMap((event) =>
      Object.entries(event.attributes)
        .filter(([key]) => key.endsWith('payload.json'))
        .map(([, value]) => [event.name.replace('iii.invocation.', ''), value] as const),
    )
  if (payloads.length === 0) return null
  return (
    <CollapsibleCard className="sentinel-ui-disclosure">
      <CollapsibleCardTrigger>
        <span className="sentinel-ui-disclosure-head">
          <span>Payloads</span>
          <span className="sentinel-ui-card-note">what the function was called with, and what it returned</span>
          <ChevronDown size={16} className="sentinel-ui-disclosure-chevron" aria-hidden="true" />
        </span>
      </CollapsibleCardTrigger>
      <CollapsibleCardContent>
        <div className="sentinel-ui-payloads">
          {payloads.map(([name, value]) => (
            <div key={name}>
              <span className="sentinel-ui-kv-key">{name}</span>
              <CodeHighlight
                code={value.length > VALUE_PREVIEW ? `${value.slice(0, VALUE_PREVIEW)}\n…` : value}
                language="json"
                wrap
              />
            </div>
          ))}
        </div>
      </CollapsibleCardContent>
    </CollapsibleCard>
  )
}

function TraceLogs({ bundle }: { bundle: EvidenceBundle }) {
  return (
    <Card>
      <CardHeader>
        <span>Logs in this trace</span>
        <span className="sentinel-ui-card-note sentinel-ui-mono">{bundle.logs.length}</span>
      </CardHeader>
      <CardBody>
        <ol className="sentinel-ui-logs">
          {bundle.logs.map((log, index) => (
            <li key={index}>
              <span className="sentinel-ui-quiet">{clock(log.timestamp_unix_nano)}</span>
              <span className="sentinel-ui-log-level">{log.severity_text}</span>
              <span className="sentinel-ui-log-body">{log.body}</span>
            </li>
          ))}
        </ol>
      </CardBody>
    </Card>
  )
}

function LogRecord({ bundle, captured }: { bundle: EvidenceBundle; captured: string }) {
  const record = bundle.logs[0]
  const attributes = Object.entries(record?.attributes ?? {})
  const target = record?.attributes['code.function'] ?? record?.attributes.target ?? record?.attributes.function_id
  return (
    <Card>
      <CardHeader>
        <span>Log record</span>
        <span className="sentinel-ui-card-note sentinel-ui-mono">{captured}</span>
        {bundle.spans.length === 0 ? (
          <Chip tone="warning" className="sentinel-ui-card-end">
            no error span in this trace
          </Chip>
        ) : null}
      </CardHeader>
      <CardBody className="sentinel-ui-origin">
        <dl className="sentinel-ui-kv">
          <dt>timestamp</dt>
          <dd>{record ? stamp(Math.floor(record.timestamp_unix_nano / 1e6)) : '—'}</dd>
          <dt>severity</dt>
          <dd>
            <span className="sentinel-ui-alert">{record?.severity_text ?? '—'}</span>
          </dd>
          <dt>service</dt>
          <dd>{bundle.worker.service_name}</dd>
          <dt>target</dt>
          <dd>{target ?? '—'}</dd>
        </dl>
        <dl className="sentinel-ui-kv">
          <dt>trace_id</dt>
          <dd>{bundle.trace_id ? short(bundle.trace_id) : '—'}</dd>
          <dt>span_id</dt>
          <dd>{record?.span_id ?? '—'}</dd>
        </dl>
        {record ? <pre className="sentinel-ui-exception">{record.body}</pre> : null}
        {attributes.length > 0 ? (
          <div className="sentinel-ui-attributes">
            {attributes.map(([key, value]) => (
              <Chip key={key} className="sentinel-ui-mono">
                {key} = {value}
              </Chip>
            ))}
          </div>
        ) : null}
        <p className="sentinel-ui-card-foot">
          This group comes from the engine's log trigger. With no span to anchor it, the fingerprint is
          the worker, the target and the normalized message — and the agent gets the surrounding log
          window instead of a trace.
        </p>
      </CardBody>
    </Card>
  )
}

function CopyTrace({ traceId }: { traceId: string }) {
  const { state, copy } = useCopyFlash(traceId, 1500)
  if (!traceId) return null
  return (
    <Button size="sm" variant="ghost" className="sentinel-ui-card-end" onClick={copy}>
      {state === 'copied' ? <Check size={16} /> : <Copy size={16} />}
      {state === 'copied' ? 'Copied' : state === 'failed' ? 'Copy failed' : 'Copy trace id'}
    </Button>
  )
}

function exceptionOf(span: EvidenceSpan | undefined): Record<string, string> {
  return span?.events.find((event) => event.name === 'exception')?.attributes ?? {}
}

function isError(span: EvidenceSpan): boolean {
  return span.status.toLowerCase() === 'error'
}

function duration(span: EvidenceSpan): string {
  if (!span.end_time_unix_nano || !span.start_time_unix_nano) return ''
  const ms = (span.end_time_unix_nano - span.start_time_unix_nano) / 1e6
  if (ms < 1) return `${Math.round(ms * 1000)} µs`
  if (ms < 1000) return `${ms.toFixed(1)} ms`
  return `${(ms / 1000).toFixed(2)} s`
}

function short(id: string): string {
  return id.length > 16 ? `${id.slice(0, 12)}…${id.slice(-4)}` : id
}

function clock(unixNano: number): string {
  return stamp(Math.floor(unixNano / 1e6)).slice(11)
}
