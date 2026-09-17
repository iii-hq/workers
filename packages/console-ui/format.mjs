/**
 * Pure, React-free formatting helpers shared by the Console and injected
 * worker UI. Bundleable: import from `@iii-dev/console-ui/format`.
 */

/**
 * Milliseconds out of anything a worker hands us. Numbers below 1e12 are
 * unix seconds (the boundary is 2001-09 in ms, year ~33k in seconds);
 * numeric strings follow the same rule; every other string is `Date.parse`d.
 */
function toMillis(input) {
  if (input instanceof Date) return input.getTime()
  if (typeof input === 'string') {
    const trimmed = input.trim()
    if (/^\d+(\.\d+)?$/.test(trimmed)) return toMillis(Number(trimmed))
    return Date.parse(trimmed)
  }
  if (typeof input !== 'number') return Number.NaN
  return input < 1_000_000_000_000 ? input * 1000 : input
}

/**
 * Short elapsed copy: `just now`, `42s`, `5m`, `3h`, `2d`, `4mo`, `1y`.
 * Compose it yourself (`${formatRelative(t)} ago`). Empty for unparsable
 * input; the future clamps to `just now`.
 */
export function formatRelative(input, now = Date.now()) {
  const ms = toMillis(input)
  if (!Number.isFinite(ms)) return ''
  const s = Math.max(0, Math.floor((now - ms) / 1000))
  if (s < 10) return 'just now'
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m`
  const h = Math.floor(m / 60)
  if (h < 24) return `${h}h`
  const d = Math.floor(h / 24)
  if (d < 30) return `${d}d`
  if (d < 365) return `${Math.floor(d / 30)}mo`
  return `${Math.floor(d / 365)}y`
}

const pad2 = (n) => String(n).padStart(2, '0')

/** `842ms`, `1.4s`, `2m 05s`, `1h 12m`. Negative or NaN reads `0ms`. */
export function formatDuration(ms) {
  if (!Number.isFinite(ms) || ms < 0) return '0ms'
  if (ms < 1000) return `${Math.round(ms)}ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`
  const totalSeconds = Math.floor(ms / 1000)
  const minutes = Math.floor(totalSeconds / 60)
  if (minutes < 60) return `${minutes}m ${pad2(totalSeconds % 60)}s`
  return `${Math.floor(minutes / 60)}h ${pad2(minutes % 60)}m`
}

const BYTE_UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB']

/** Binary units with a space: `512 B`, `1.0 KiB`, `3.2 MiB`; `—` for null. */
export function formatBytes(n) {
  if (n == null || !Number.isFinite(n)) return '—'
  let value = Math.max(0, n)
  if (value < 1024) return `${Math.round(value)} B`
  let unit = 0
  while (value >= 1024 && unit < BYTE_UNITS.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value.toFixed(1)} ${BYTE_UNITS[unit]}`
}

/* ── harness envelope ───────────────────────────────────────────────── */

/**
 * The harness wraps every tool result in `{ content: ContentBlock[],
 * details, terminate }` before relaying it; the console sees the same
 * shape on the function_call output stream. This peels the wrapper so a
 * renderer works on the flat response. Idempotent: an already-flat payload
 * comes back unchanged. The discriminator is `Array.isArray(content)` plus
 * a `details` key — what the harness sets unconditionally.
 */
export function unwrapEnvelope(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return value
  if (Array.isArray(value.content) && 'details' in value) return value.details
  return value
}

/* ── errors ─────────────────────────────────────────────────────────── */

/** Codes that only say "the call failed" — never worth showing. */
const TRANSPORT_CODES = new Set(['invocation_failed', 'handler_error'])
const HANDLER_PREFIX = /^handler error: /

const str = (v) => (typeof v === 'string' && v.length > 0 ? v : undefined)

/**
 * The wire error object a rejected `iii.trigger` throws, normalised: the
 * human text lives under `message`, `error`, `reason` or `detail` (an
 * `error` object nests one level further).
 */
function bodyOf(value) {
  if (typeof value !== 'object' || value === null) return null
  const { code, inner_code, message, error, reason, detail } = value
  const nested = typeof error === 'object' && error !== null ? bodyOf(error) : null
  return {
    code: str(code) ?? nested?.code,
    inner_code: str(inner_code) ?? nested?.inner_code,
    message: str(message) ?? str(error) ?? nested?.message ?? str(reason) ?? str(detail),
  }
}

/**
 * Peel `handler error: {json}` wrappers until the message stops being one.
 * Bounded, because a malformed payload must not spin.
 */
function unwrap(body) {
  let current = body
  for (let depth = 0; depth < 4; depth++) {
    const text = current.message
    if (!text) return current
    const brace = text.indexOf('{')
    if (brace === -1) return current
    let parsed
    try {
      parsed = JSON.parse(text.slice(brace))
    } catch {
      return current
    }
    const inner = bodyOf(parsed)
    if (!inner?.message) return current
    current = inner
  }
  return current
}

/** The innermost error code a rejected trigger carries (outer code as fallback). */
export function errorCode(err) {
  const body = bodyOf(err)
  if (!body) return undefined
  return unwrap(body).code ?? body.code
}

/**
 * One readable string out of whatever a rejected `iii.trigger` throws.
 * The browser SDK rejects with the wire error *object*, so a naive
 * `String(err)` renders `[object Object]`; this unwraps the envelope, keeps
 * the handler's own code (`D214: …`) and drops transport-only ones.
 */
export function errorMessage(err) {
  if (err instanceof Error) return err.message || err.name || 'Unknown error'
  if (typeof err === 'string') return err || 'Unknown error'
  if (err == null) return 'Unknown error'

  const body = bodyOf(err)
  if (body?.message) {
    const { code, inner_code, message: unwrapped } = unwrap(body)
    // A plain-prose handler error (`handler error: D214 invalid_input: …`)
    // has no JSON to peel, so the SDK prefix survives — strip it here.
    const message = unwrapped?.replace(HANDLER_PREFIX, '')
    const label = code && !TRANSPORT_CODES.has(code) ? code : null
    if (!label) return message ?? 'Unknown error'
    return `${label}${inner_code ? ` (${inner_code})` : ''}: ${message}`
  }

  if (typeof err === 'object') {
    try {
      return JSON.stringify(err) ?? 'Unknown error'
    } catch {
      return 'Unknown error'
    }
  }
  return String(err)
}

/* ── clipboard ──────────────────────────────────────────────────────── */

function execCommandFallback(text) {
  if (typeof document === 'undefined') return false
  const prior = document.activeElement
  const textarea = document.createElement('textarea')
  textarea.value = text
  // Off-screen but selectable; readonly keeps mobile keyboards closed.
  textarea.setAttribute('readonly', '')
  textarea.style.position = 'fixed'
  textarea.style.left = '-9999px'
  document.body.appendChild(textarea)
  try {
    textarea.select()
    return document.execCommand('copy')
  } catch {
    return false
  } finally {
    textarea.remove()
    if (typeof HTMLElement !== 'undefined' && prior instanceof HTMLElement) {
      prior.focus()
    }
  }
}

/**
 * Clipboard write that survives insecure origins (`http://<LAN-IP>`, where
 * `navigator.clipboard` is undefined): async API first, hidden-textarea
 * `execCommand('copy')` second. True only when a strategy succeeded; never
 * throws.
 */
export async function copyText(text) {
  if (typeof navigator !== 'undefined' && navigator?.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text)
      return true
    } catch {
      // Permissions can reject even on secure origins — try the fallback.
    }
  }
  return execCommandFallback(text)
}
