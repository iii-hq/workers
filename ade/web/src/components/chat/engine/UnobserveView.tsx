import { Chip, MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import { JsonHighlight } from '@/lib/syntax'
import {
  disposeObserverResponseSchema,
  safeParseRequest,
  safeParseResponse,
  type UnobserveRequest,
  unobserveRequestSchema,
  unobserveResponseSchema,
} from './parsers'

/**
 * `harness::unobserve` (the agent removing its own observer) and
 * `engine::observers::dispose` (removing any observer by id): both take
 * `{ id }` and say whether an observer was removed.
 */
export function UnobserveView({
  input,
  output,
  running,
}: {
  input: unknown
  output: unknown
  running?: boolean
}) {
  const request = safeParseRequest<UnobserveRequest>(
    unobserveRequestSchema,
    input,
  )
  if (!request) return <RawValue label="request" value={input} />

  const removed = running ? undefined : removedOf(output)
  const status = running
    ? 'removing observer…'
    : removed === true
      ? 'observer manually removed'
      : removed === false
        ? 'observer not found'
        : 'observer removal failed'
  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={status}
          variant={
            running
              ? 'default'
              : removed === true
                ? 'accent'
                : removed === false
                  ? 'default'
                  : 'warn'
          }
        />
        <Chip>
          <span className="iii-ui-eyebrow">id</span>
          <span className="ml-1 text-ink" title={request.id}>
            {shortenId(request.id)}
          </span>
        </Chip>
      </MetaRow>
      <div className="bg-warn-muted px-3 py-2 font-mono text-[11px] text-ink-faint">
        {removed === true
          ? 'This observer was explicitly removed and will not fire again.'
          : removed === false
            ? 'No observer matched this id.'
            : running
              ? 'Removing this observer.'
              : 'The call did not return a valid removal result.'}
      </div>
    </div>
  )
}

/** `{ removed }` from the harness, `{ disposed }` from the engine. */
function removedOf(output: unknown): boolean | undefined {
  const unobserved = safeParseResponse(unobserveResponseSchema, output)
  if (unobserved) return unobserved.removed
  return safeParseResponse(disposeObserverResponseSchema, output)?.disposed
}

function RawValue({ label, value }: { label: string; value: unknown }) {
  const json = JSON.stringify(value, null, 2) ?? String(value)
  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="border-b border-rule-2 bg-paper-2 px-3 py-1.5 iii-ui-eyebrow">
        {label}
      </div>
      <JsonHighlight code={json} wrap />
    </div>
  )
}

function shortenId(id: string): string {
  return id.length <= 18 ? id : `${id.slice(0, 10)}…${id.slice(-5)}`
}
