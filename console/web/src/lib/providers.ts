/**
 * Typed wrappers over the iii bus for provider auth and config functions.
 * Mirrors the pattern in `src/lib/models-catalog.ts`.
 *
 * NEVER call `auth::get_token` from the browser -- it returns the raw
 * credential and would leak the key. Use `auth::status` for masked display.
 */

import { getIiiClient } from '@/lib/iii-client'

/* ------------------------------------------------------------------ */
/*  auth-credentials bus functions                                     */
/* ------------------------------------------------------------------ */

export interface AuthStatusResult {
  configured: boolean
  source?: 'stored' | 'environment'
  label?: string
}

export async function fetchAuthStatus(
  provider: string,
): Promise<AuthStatusResult> {
  const client = await getIiiClient()
  return client.call<AuthStatusResult>('auth::status', { provider })
}

export interface ListProvidersResult {
  providers: string[]
}

export async function fetchListProviders(): Promise<ListProvidersResult> {
  const client = await getIiiClient()
  return client.call<ListProvidersResult>('auth::list_providers', {})
}

export interface SetTokenPayload {
  provider: string
  credential: { type: 'api_key'; key: string }
}

export async function setToken(
  payload: SetTokenPayload,
): Promise<{ ok: true }> {
  const client = await getIiiClient()
  return client.call<{ ok: true }>(
    'auth::set_token',
    payload as unknown as Record<string, unknown>,
  )
}

export async function deleteToken(provider: string): Promise<{ ok: true }> {
  const client = await getIiiClient()
  return client.call<{ ok: true }>('auth::delete_token', { provider })
}

/* ------------------------------------------------------------------ */
/*  provider-config bus functions                                      */
/* ------------------------------------------------------------------ */

export interface ProviderConfigResult {
  default_api_url?: string
  default_max_tokens?: number
}

export async function fetchProviderConfig(
  provider: string,
): Promise<ProviderConfigResult> {
  const client = await getIiiClient()
  return client.call<ProviderConfigResult>('provider_config::get', {
    provider,
  })
}

export interface SetProviderConfigPayload {
  provider: string
  default_api_url?: string
  default_max_tokens?: number
}

export async function setProviderConfig(
  payload: SetProviderConfigPayload,
): Promise<{ ok: true }> {
  const client = await getIiiClient()
  return client.call<{ ok: true }>(
    'provider_config::set',
    payload as unknown as Record<string, unknown>,
  )
}

export async function clearProviderConfig(
  provider: string,
): Promise<{ ok: true }> {
  const client = await getIiiClient()
  return client.call<{ ok: true }>('provider_config::clear', { provider })
}

export interface ProviderConfigListEntry {
  provider: string
  overrides: { default_api_url?: string; default_max_tokens?: number }
}

export async function fetchProviderConfigList(): Promise<{
  entries: ProviderConfigListEntry[]
}> {
  const client = await getIiiClient()
  return client.call<{ entries: ProviderConfigListEntry[] }>(
    'provider_config::list',
    {},
  )
}

/* ------------------------------------------------------------------ */
/*  Validation helpers (client-side error prevention)                  */
/* ------------------------------------------------------------------ */

export const MAX_TOKENS_LIMIT = 1_048_576

export type ValidationResult<T = void> =
  | { ok: true; value: T }
  | { ok: false; error: string }

/**
 * Validation error strings include the recovery hint inline (e.g.
 * "not a valid url — include http:// or https:// and the full path").
 * This matches the message+hint pattern used by per-mutation errors so
 * the operator gets the same level of recovery guidance whether the
 * failure was client-side validation or a server-side mutation.
 *
 * Precondition: callers pass a non-empty trimmed string. An empty
 * value is the operator's signal that they want the provider default
 * (no override) — save() correctly skips the mutation in that case
 * rather than calling validate. We therefore do not return a
 * contradictory "required — clear the field" error for empty input;
 * empty input is simply not a validation concern.
 */
export function validateApiUrl(raw: string): ValidationResult<string> {
  const trimmed = raw.trim()
  let parsed: URL
  try {
    parsed = new URL(trimmed)
  } catch {
    return {
      ok: false,
      error:
        'not a valid url — include http:// or https:// and the full path (e.g. http://localhost:1234/v1/chat/completions)',
    }
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    return {
      ok: false,
      error: `must use http or https (got ${parsed.protocol.replace(':', '')})`,
    }
  }
  return { ok: true, value: trimmed }
}

/**
 * Precondition matches `validateApiUrl`: callers gate on non-empty.
 * Empty input means "use the provider default cap" and is not a
 * validation concern.
 */
export function validateMaxTokens(raw: string): ValidationResult<number> {
  const trimmed = raw.trim()
  const n = Number(trimmed)
  if (!Number.isInteger(n)) {
    return { ok: false, error: 'must be a whole number (no decimals)' }
  }
  if (n <= 0) {
    return { ok: false, error: 'must be at least 1' }
  }
  if (n > MAX_TOKENS_LIMIT) {
    return {
      ok: false,
      error: `must be ≤ ${MAX_TOKENS_LIMIT.toLocaleString()} (the harness ceiling)`,
    }
  }
  return { ok: true, value: n }
}

/**
 * Strip noise from raw error messages surfaced by the bus / SDK so the UI
 * renders something quiet and terminal-shaped rather than `Error: foo`.
 *
 * Handles three shapes:
 *   1. `Error` instances -> `.message`
 *   2. iii-browser-sdk bus rejections -> plain object `{ code, message, stacktrace? }`
 *      surfaced as-is from `onInvocationResult`. We render `"<code>: <message>"`
 *      (or just one if the other is missing).
 *   3. Anything else -> `String(err)`, then strip prefixes.
 *
 * Without the object branch, plain bus errors stringified to "[object Object]"
 * and the UI showed `error: [object object]` (lowercased by the .toLowerCase
 * step below) -- which hid the actual failure from the user.
 */
export function normalizeErrorMessage(err: unknown): string {
  if (!err) return 'unknown error'
  let raw: string
  if (err instanceof Error) {
    raw = err.message
  } else if (typeof err === 'object') {
    const o = err as { code?: unknown; message?: unknown; error?: unknown }
    const code = typeof o.code === 'string' ? o.code : ''
    const message =
      typeof o.message === 'string'
        ? o.message
        : typeof o.error === 'string'
          ? o.error
          : ''
    if (code && message) raw = `${code}: ${message}`
    else if (message) raw = message
    else if (code) raw = code
    else raw = String(err)
  } else {
    raw = String(err)
  }
  return raw
    .replace(/^Error:\s*/i, '')
    .replace(/^@fn\([^)]+\)\s*/, '')
    .trim()
    .toLowerCase()
}
