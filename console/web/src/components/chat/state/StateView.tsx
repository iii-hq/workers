import { FilterChip } from '@/components/chat/engine/shared'
import { MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import { JsonHighlight } from '@/lib/syntax'
import { safeParseRequest, stateRequestSchema, unwrapEnvelope } from './parsers'

interface StateViewProps {
  functionId: string
  input: unknown
  output: unknown
  running?: boolean
}

/** `null`, `undefined`, `""`, `[]`, `{}` render as a compact "empty" note
 * rather than a noisy JSON block. Mirrors `ValuePane`'s `isEmptyValue`. */
function isEmptyValue(v: unknown): boolean {
  if (v === null || v === undefined) return true
  if (typeof v === 'string') return v.length === 0
  if (Array.isArray(v)) return v.length === 0
  if (typeof v === 'object') {
    return Object.keys(v as Record<string, unknown>).length === 0
  }
  return false
}

export function StateView({
  functionId,
  input,
  output,
  running,
}: StateViewProps) {
  const req = safeParseRequest(stateRequestSchema, input)
  const op = functionId.startsWith('state::')
    ? functionId.slice('state::'.length)
    : functionId

  const value = running ? undefined : unwrapEnvelope(output)
  const empty = !running && isEmptyValue(value)

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label={op} variant={running ? 'default' : 'accent'} />
        {req?.scope ? <FilterChip label="scope" value={req.scope} /> : null}
        {req?.key ? <FilterChip label="key" value={req.key} /> : null}
      </MetaRow>
      {running ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · running…
        </div>
      ) : empty ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          · empty
        </div>
      ) : (
        <JsonHighlight code={JSON.stringify(value ?? null, null, 2)} />
      )}
    </div>
  )
}
