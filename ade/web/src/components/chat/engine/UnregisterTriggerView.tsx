import { Chip, MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import { JsonHighlight } from '@/lib/syntax'
import {
  safeParseRequest,
  safeParseResponse,
  type UnregisterTriggerRequest,
  type UnregisterTriggerResponse,
  unregisterTriggerRequestSchema,
  unregisterTriggerResponseSchema,
} from './parsers'
import { FilterChip } from './shared'

export function UnregisterTriggerView({
  input,
  output,
  running,
}: {
  input: unknown
  output: unknown
  running?: boolean
}) {
  const request = safeParseRequest<UnregisterTriggerRequest>(
    unregisterTriggerRequestSchema,
    input,
  )
  if (!request) return <RawValue label="request" value={input} />

  const response = running
    ? null
    : safeParseResponse<UnregisterTriggerResponse>(
        unregisterTriggerResponseSchema,
        output,
      )
  const status = running
    ? 'removing binding…'
    : response?.removed === true
      ? 'binding manually removed'
      : response?.removed === false
        ? 'binding not found'
        : 'binding removal failed'
  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={status}
          variant={
            running
              ? 'default'
              : response?.removed === true
                ? 'accent'
                : response?.removed === false
                  ? 'default'
                  : 'warn'
          }
        />
        {request.trigger_type ? (
          <FilterChip label="type" value={request.trigger_type} />
        ) : null}
        <Chip>
          <span className="tracking-[0.06em] text-ink-faint uppercase">id</span>
          <span className="ml-1 text-ink" title={request.id}>
            {shortenId(request.id)}
          </span>
        </Chip>
      </MetaRow>
      <div className="border-l-2 border-l-warn px-3 py-2 font-mono text-[11px] text-ink-faint">
        {response?.removed === true
          ? 'This binding was explicitly removed and will not fire again.'
          : response?.removed === false
            ? 'No active binding matched this id.'
            : running
              ? 'Removing this binding from the trigger registry.'
              : 'The engine did not return a valid removal result.'}
      </div>
    </div>
  )
}

function RawValue({ label, value }: { label: string; value: unknown }) {
  const json = JSON.stringify(value, null, 2) ?? String(value)
  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="border-b border-rule-2 bg-paper-2 px-3 py-1.5 font-mono text-[11px] tracking-[0.06em] text-ink-faint uppercase">
        {label}
      </div>
      <JsonHighlight code={json} wrap />
    </div>
  )
}

function shortenId(id: string): string {
  return id.length <= 18 ? id : `${id.slice(0, 10)}…${id.slice(-5)}`
}
