/* Error-display normalisation for every failure shape the console may
   hand a shell renderer: flat wire errors, harness `{ content, details }`
   wrappers, translate `{ error: { kind, message, … } }` envelopes,
   approval denials, and fail-closed dispatch-policy denials. Ported from
   the console's sandbox/parsers.ts when the shell function-trigger family
   moved into this worker's injected UI.

   Deliberately React-free (the cards live in errors.tsx): parsing stays
   importable from parsers.ts and its node-run tests without touching
   `@iii-dev/console-ui`, whose module entry throws outside the console. */

import { z } from 'zod'
import {
  collectErrorCandidates,
  contentBlocksText,
  extractFirstJsonObject,
  unwrapEnvelope,
} from './envelope'

/** The Stripe-style flat error payload (sandbox-daemon errors.rs shape) —
    shell forwards these verbatim for sandbox-target calls. */
export const errorWireSchema = z.object({
  type: z.string(),
  code: z.string().regex(/^S\d{3}$/),
  message: z.string(),
  docs_url: z.string().optional(),
  retryable: z.boolean().optional(),
  fix: z.unknown().optional(),
  fix_note: z.string().nullable().optional(),
})
export type ErrorWire = z.infer<typeof errorWireSchema>

const denialEnvelopeSchema = z.object({
  schema_version: z.number().optional(),
  status: z.string().optional(),
  denied_by: z.string().optional(),
  function_id: z.string().optional(),
  reason: z.string().optional(),
})
export type DenialEnvelopeWire = z.infer<typeof denialEnvelopeSchema>

const functionErrorEnvelopeSchema = z.object({
  kind: z.string(),
  message: z.string(),
  details: z.unknown().optional(),
  content: z.array(z.unknown()).optional(),
})

export type InvocationError = {
  title: string
  message: string
  functionId?: string
  deniedBy?: string
  reason?: string
  detailText?: string
}

/**
 * A fail-closed dispatch-policy denial: the calling agent's `functions`
 * allow-list does not cover the function it tried. Distinct from a
 * permission denial (approval gate) — structural, not retryable.
 */
export interface DispatchDenial {
  /** The blocked function id, when the message names it. */
  functionId?: string
  /** Namespace prefix of `functionId` (e.g. `shell` from `shell::exec`). */
  namespace?: string
  /** The raw engine message, kept verbatim under the actionable hint. */
  message: string
}

export type ErrorDisplay =
  | { variant: 'wire'; error: ErrorWire }
  | { variant: 'invocation'; error: InvocationError }
  | { variant: 'dispatch-denied'; error: DispatchDenial }

function tryParseWire(value: unknown): ErrorWire | null {
  const parsed = errorWireSchema.safeParse(unwrapEnvelope(value))
  if (parsed.success) return parsed.data

  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const obj = value as Record<string, unknown>
  if (
    obj.error === 'handler_error' ||
    (typeof obj.code === 'string' && /^S\d{3}$/.test(obj.code))
  ) {
    const { error: _tag, ...rest } = obj
    const tagged = errorWireSchema.safeParse(rest)
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
): InvocationError {
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
    denial.reason ?? fallbackMessage ?? 'The trigger could not complete.'
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
): InvocationError | null {
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

/** Engine fail-closed dispatch-policy denial signature (harness policy.rs). */
const DISPATCH_DENIAL_RE = /not permitted by this agent.?s dispatch policy/i
/** `function <id> is not permitted …` — captures the blocked function id. */
const DISPATCH_FN_RE = /function\s+([A-Za-z0-9_:-]+)\s+is not permitted/i

/** A message-like string from a candidate, for signature scanning. */
function candidateText(candidate: unknown): string | undefined {
  if (typeof candidate === 'string') return candidate
  if (candidate && typeof candidate === 'object' && !Array.isArray(candidate)) {
    const msg = (candidate as Record<string, unknown>).message
    if (typeof msg === 'string') return msg
  }
  return undefined
}

export function parseDispatchDenial(value: unknown): DispatchDenial | null {
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

/**
 * Normalise every generic failure shape (the shell-specific S-code text
 * scan lives in the family's parsers, layered on top of this).
 */
export function parseErrorDisplay(value: unknown): ErrorDisplay | null {
  const candidates = collectErrorCandidates(value)

  // Dispatch-policy denial is the most specific + actionable shape —
  // detect it before the generic invocation-failed fallback swallows it.
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
      return {
        variant: 'invocation',
        error: denialToInvocation(denial),
      }
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

