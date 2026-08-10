/**
 * Sandbox error decoding — the daemon's Stripe-style flat error payload
 * (`{ type, code: "S###", message, fix_note?, retryable?, docs_url? }`,
 * errors.rs). By the time a failed trigger reaches the page it is usually
 * an `Error` whose message may be the plain text, the JSON payload, or
 * prose that merely quotes an S-code — all three decode here, tolerantly.
 */

import { asRecord } from '../lib/payload'

export interface SandboxError {
  code: string | null
  message: string
  fix_note: string | null
  retryable: boolean | null
  docs_url: string | null
}

function fromRecord(rec: Record<string, unknown>): SandboxError | null {
  const code = typeof rec.code === 'string' ? rec.code : null
  const message = typeof rec.message === 'string' ? rec.message : null
  if (!code && !message) return null
  return {
    code,
    message: message ?? '',
    fix_note: typeof rec.fix_note === 'string' ? rec.fix_note : null,
    retryable: typeof rec.retryable === 'boolean' ? rec.retryable : null,
    docs_url: typeof rec.docs_url === 'string' ? rec.docs_url : null,
  }
}

export function parseSandboxError(err: unknown): SandboxError {
  // A structured error object (or one carried under `.error`).
  const rec = asRecord(err)
  const direct = rec
    ? (fromRecord(rec) ?? fromRecord(asRecord(rec.error) ?? {}))
    : null

  const message = direct?.message
    ? direct.message
    : err instanceof Error
      ? err.message
      : rec
        ? JSON.stringify(err)
        : String(err)

  // A message that IS (or embeds) the JSON payload — a structured record
  // often just relays the daemon envelope as its message text, and the
  // decoded payload must win so code/retryable/fix_note/docs_url populate.
  const jsonStart = message.indexOf('{')
  if (jsonStart >= 0) {
    try {
      const parsed = fromRecord(
        asRecord(JSON.parse(message.slice(jsonStart))) ?? {},
      )
      if (parsed?.message) return parsed
    } catch {
      /* not JSON — fall through */
    }
  }

  if (direct?.message) return direct

  // Prose quoting an S-code.
  const code = message.match(/\bS\d{3}\b/)?.[0] ?? null
  return { code, message, fix_note: null, retryable: null, docs_url: null }
}
