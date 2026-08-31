import { FilterChip } from '@/components/chat/engine/shared'
import { MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import { JsonHighlight } from '@/lib/syntax'
import {
  safeParseRequest,
  transformOp,
  unwrapEnvelope,
  utilRequestSchema,
  utilResponseSchema,
} from './parsers'

interface UtilViewProps {
  functionId: string
  input: unknown
  output: unknown
  running?: boolean
}

/** `null`, `undefined`, `""`, `[]`, `{}` render as a compact "empty" note
 * rather than a noisy JSON block. Mirrors `StateView`. */
function isEmptyValue(v: unknown): boolean {
  if (v === null || v === undefined) return true
  if (typeof v === 'string') return v.length === 0
  if (Array.isArray(v)) return v.length === 0
  if (typeof v === 'object') {
    return Object.keys(v as Record<string, unknown>).length === 0
  }
  return false
}

/** `fp::*` — the seventeen lodash-style transforms (`FP_TRANSFORM_OPS`).
    Op label + its params as chips, then the transformed value. */
export function UtilView({
  functionId,
  input,
  output,
  running,
}: UtilViewProps) {
  const req = safeParseRequest(utilRequestSchema, input)
  const op = transformOp(functionId)

  // Success details is the `UtilResponse { value }` wrapper — show the value.
  const unwrapped = running ? undefined : unwrapEnvelope(output)
  const parsed = utilResponseSchema.safeParse(unwrapped)
  const value = parsed.success ? parsed.data.value : unwrapped
  const empty = !running && isEmptyValue(value)

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label={op} variant={running ? 'default' : 'accent'} />
        {req?.path != null ? (
          <FilterChip label="path" value={req.path} />
        ) : null}
        {req?.paths?.length ? (
          <FilterChip label="paths" value={req.paths.join(', ')} />
        ) : null}
        {req?.n != null ? <FilterChip label="n" value={String(req.n)} /> : null}
        {req?.separator != null ? (
          <FilterChip label="sep" value={JSON.stringify(req.separator)} />
        ) : null}
        {req?.matches ? (
          <FilterChip label="matches" value={JSON.stringify(req.matches)} />
        ) : null}
        {req !== null && req.default !== undefined ? (
          <FilterChip label="default" value={JSON.stringify(req.default)} />
        ) : null}
      </MetaRow>
      {running ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · running…
        </div>
      ) : empty ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          · empty
        </div>
      ) : typeof value === 'string' ? (
        <pre className="overflow-x-auto whitespace-pre-wrap break-all px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink">
          {value}
        </pre>
      ) : (
        <JsonHighlight code={JSON.stringify(value ?? null, null, 2)} />
      )}
    </div>
  )
}
