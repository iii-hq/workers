/**
 * Shared-infra error parsing + view for `browser::*` chat cards. Ported from
 * the console's sandbox `parsers.ts` + `ErrorView.tsx` (the console rendered
 * browser errors through `parseSandboxErrorDisplay` / `SandboxErrorView`).
 * Normalises every failure shape the console may receive — flat Stripe-style
 * wire errors, harness `{ content, details }` wrappers, denial envelopes,
 * function_error envelopes, dispatch-policy denials, and JSON embedded in
 * strings — into one display union. The exec-stream (S200) special case from
 * the sandbox view is dropped: browser errors never carry an ExecResponse.
 * Tailwind utility classes are replaced with scoped `br-ui-*` classes.
 */

import { Badge } from '@iii-dev/console-ui'
import { z } from 'zod'

const infraErrorWireSchema = z.object({
  type: z.string(),
  code: z.string().regex(/^S\d{3}$/),
  message: z.string(),
  docs_url: z.string().optional(),
  retryable: z.boolean().optional(),
  fix: z.unknown().optional(),
  fix_note: z.string().nullable().optional(),
})
export type InfraErrorWire = z.infer<typeof infraErrorWireSchema>

const denialEnvelopeSchema = z.object({
  schema_version: z.number().optional(),
  status: z.string().optional(),
  denied_by: z.string().optional(),
  function_id: z.string().optional(),
  reason: z.string().optional(),
})
type DenialEnvelopeWire = z.infer<typeof denialEnvelopeSchema>

const functionErrorEnvelopeSchema = z.object({
  kind: z.string(),
  message: z.string(),
  details: z.unknown().optional(),
  content: z.array(z.unknown()).optional(),
})

export interface InfraInvocationError {
  title: string
  message: string
  functionId?: string
  deniedBy?: string
  reason?: string
  detailText?: string
}

export interface InfraDispatchDenial {
  functionId?: string
  namespace?: string
  message: string
}

export type InfraErrorDisplay =
  | { variant: 'wire'; error: InfraErrorWire }
  | { variant: 'invocation'; error: InfraInvocationError }
  | { variant: 'dispatch-denied'; error: InfraDispatchDenial }

/** `{ content: [...], details }` harness result envelope → details. */
function unwrapEnvelope(value: unknown): unknown {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return value
  const obj = value as Record<string, unknown>
  if (Array.isArray(obj.content) && 'details' in obj) return obj.details
  return value
}

function contentBlocksText(content: unknown): string | undefined {
  if (!Array.isArray(content)) return undefined
  const parts: string[] = []
  for (const block of content) {
    if (!block || typeof block !== 'object') continue
    const obj = block as Record<string, unknown>
    if (
      obj.type === 'text' &&
      typeof obj.text === 'string' &&
      obj.text.length > 0
    ) {
      parts.push(obj.text)
    }
  }
  return parts.length > 0 ? parts.join('\n') : undefined
}

function extractFirstJsonObject(text: string): unknown | null {
  const start = text.indexOf('{')
  if (start === -1) return null
  let depth = 0
  for (let i = start; i < text.length; i++) {
    const ch = text[i]
    if (ch === '{') depth++
    else if (ch === '}') {
      depth--
      if (depth === 0) {
        try {
          return JSON.parse(text.slice(start, i + 1))
        } catch {
          return null
        }
      }
    }
  }
  return null
}

function tryParseWire(value: unknown): InfraErrorWire | null {
  const parsed = infraErrorWireSchema.safeParse(unwrapEnvelope(value))
  if (parsed.success) return parsed.data

  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const obj = value as Record<string, unknown>
  if (
    obj.error === 'handler_error' ||
    (typeof obj.code === 'string' && /^S\d{3}$/.test(obj.code))
  ) {
    const { error: _tag, ...rest } = obj
    const tagged = infraErrorWireSchema.safeParse(rest)
    if (tagged.success) return tagged.data
  }
  return null
}

function tryParseDenial(value: unknown): DenialEnvelopeWire | null {
  const parsed = denialEnvelopeSchema.safeParse(unwrapEnvelope(value))
  if (!parsed.success) return null
  if (parsed.data.status !== 'denied' && !parsed.data.denied_by) return null
  return parsed.data
}

function denialToInvocation(
  denial: DenialEnvelopeWire,
  fallbackMessage?: string,
  detailText?: string,
): InfraInvocationError {
  const deniedBy = denial.denied_by
  const title =
    deniedBy === 'gate_unavailable'
      ? 'Gate unavailable'
      : deniedBy === 'permissions'
        ? 'Permission denied'
        : deniedBy === 'user'
          ? 'Denied by user'
          : denial.status === 'denied'
            ? 'Denied'
            : 'Trigger failed'
  const message =
    denial.reason ?? fallbackMessage ?? 'The browser trigger could not complete.'
  return {
    title,
    message,
    functionId: denial.function_id,
    deniedBy,
    reason: denial.reason,
    detailText,
  }
}

function invocationFromFunctionError(
  envelope: z.infer<typeof functionErrorEnvelopeSchema>,
): InfraInvocationError | null {
  const detailText = contentBlocksText(envelope.content)
  const denial =
    envelope.details != null ? tryParseDenial(envelope.details) : null
  if (denial) {
    return denialToInvocation(denial, envelope.message, detailText)
  }
  return {
    title: 'Trigger failed',
    message: envelope.message,
    detailText,
  }
}

function collectErrorCandidates(value: unknown): unknown[] {
  const seen = new Set<unknown>()
  const out: unknown[] = []
  const push = (candidate: unknown) => {
    if (seen.has(candidate)) return
    seen.add(candidate)
    out.push(candidate)
  }

  push(value)
  push(unwrapEnvelope(value))

  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const obj = value as Record<string, unknown>
    if (obj.error && typeof obj.error === 'object') {
      const err = obj.error as Record<string, unknown>
      push(err)
      if ('details' in err) push(err.details)
      if (typeof err.message === 'string') push(err.message)
      const text = contentBlocksText(err.content)
      if (text) push(text)
    }
  }

  return out
}

const DISPATCH_DENIAL_RE = /not permitted by this agent.?s dispatch policy/i
const DISPATCH_FN_RE = /function\s+([A-Za-z0-9_:-]+)\s+is not permitted/i

function candidateText(candidate: unknown): string | undefined {
  if (typeof candidate === 'string') return candidate
  if (candidate && typeof candidate === 'object' && !Array.isArray(candidate)) {
    const msg = (candidate as Record<string, unknown>).message
    if (typeof msg === 'string') return msg
  }
  return undefined
}

function parseDispatchDenial(value: unknown): InfraDispatchDenial | null {
  for (const candidate of collectErrorCandidates(value)) {
    const text = candidateText(candidate)
    if (!text || !DISPATCH_DENIAL_RE.test(text)) continue
    const functionId = text.match(DISPATCH_FN_RE)?.[1]
    const namespace = functionId?.includes('::')
      ? functionId.split('::')[0]
      : undefined
    return { functionId, namespace, message: text }
  }
  return null
}

export function parseInfraErrorDisplay(
  value: unknown,
): InfraErrorDisplay | null {
  const candidates = collectErrorCandidates(value)

  const dispatchDenial = parseDispatchDenial(value)
  if (dispatchDenial) {
    return { variant: 'dispatch-denied', error: dispatchDenial }
  }

  for (const candidate of candidates) {
    const wire = tryParseWire(candidate)
    if (wire) return { variant: 'wire', error: wire }

    if (typeof candidate === 'string') {
      const embedded = extractFirstJsonObject(candidate)
      if (embedded != null) {
        const wireFromText = tryParseWire(embedded)
        if (wireFromText) return { variant: 'wire', error: wireFromText }
      }
    }
  }

  for (const candidate of candidates) {
    const denial = tryParseDenial(candidate)
    if (denial) {
      return { variant: 'invocation', error: denialToInvocation(denial) }
    }
  }

  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const parsed = functionErrorEnvelopeSchema.safeParse(
      (value as Record<string, unknown>).error,
    )
    if (parsed.success && parsed.data.kind === 'function_error') {
      const invocation = invocationFromFunctionError(parsed.data)
      if (invocation) {
        return { variant: 'invocation', error: invocation }
      }
    }
  }

  return null
}

function WireErrorView({ error }: { error: InfraErrorWire }) {
  return (
    <div className="br-ui-err">
      <div className="br-ui-err-head">
        <Badge variant="warn" className="br-ui-err-code">
          {error.code}
        </Badge>
        <span className="br-ui-err-type">{error.type}</span>
        {error.retryable === true ? (
          <Badge variant="accent" className="br-ui-pill">
            retryable
          </Badge>
        ) : null}
      </div>
      <pre className="br-ui-err-msg">
        <code>{error.message}</code>
      </pre>
      {error.fix_note ? (
        <div className="br-ui-err-note">{error.fix_note}</div>
      ) : null}
      {error.docs_url ? (
        <a
          href={error.docs_url}
          target="_blank"
          rel="noreferrer noopener"
          className="br-ui-err-docs"
        >
          docs ↗
        </a>
      ) : null}
    </div>
  )
}

function InvocationErrorView({ error }: { error: InfraInvocationError }) {
  const badge = error.deniedBy ?? 'error'
  const showDetailText =
    error.detailText &&
    error.detailText !== error.message &&
    error.detailText !== error.reason
  return (
    <div className="br-ui-err">
      <div className="br-ui-err-head">
        <Badge variant="warn" className="br-ui-err-code">
          {badge}
        </Badge>
        <span className="br-ui-err-type">{error.title}</span>
      </div>
      {error.functionId ? (
        <div className="br-ui-err-note">
          function <code className="br-ui-code">{error.functionId}</code>
        </div>
      ) : null}
      <pre className="br-ui-err-msg">
        <code>{error.message}</code>
      </pre>
      {showDetailText ? (
        <pre className="br-ui-err-detail">
          <code>{error.detailText}</code>
        </pre>
      ) : null}
    </div>
  )
}

function DispatchDeniedView({ denial }: { denial: InfraDispatchDenial }) {
  const fn = denial.functionId
  return (
    <div className="br-ui-err">
      <div className="br-ui-err-head">
        <Badge variant="warn" className="br-ui-err-code">
          denied
        </Badge>
        <span className="br-ui-err-type">dispatch policy</span>
      </div>
      {fn ? (
        <div className="br-ui-err-note">
          blocked <code className="br-ui-code">{fn}</code>
        </div>
      ) : null}
      <div className="br-ui-err-body">
        This agent's allow-list doesn't cover{' '}
        {fn ? <code className="br-ui-code">{fn}</code> : 'this function'}. Grant
        it where the agent is defined (a workflow node's{' '}
        <code className="br-ui-code">agent.functions</code> / the def's{' '}
        <code className="br-ui-code">default_functions</code>, or a session's{' '}
        <code className="br-ui-code">options.functions.allow</code>).
      </div>
      <pre className="br-ui-err-detail">
        <code>{denial.message}</code>
      </pre>
    </div>
  )
}

export function InfraErrorView({ display }: { display: InfraErrorDisplay }) {
  if (display.variant === 'wire') {
    return <WireErrorView error={display.error} />
  }
  if (display.variant === 'dispatch-denied') {
    return <DispatchDeniedView denial={display.error} />
  }
  return <InvocationErrorView error={display.error} />
}
