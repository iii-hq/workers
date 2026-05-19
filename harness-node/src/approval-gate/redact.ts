/**
 * Recursive argument redaction for denial envelopes. Pure and immutable:
 * never mutates the input. Lives inside the approval-gate worker (not in
 * a shared utility) on purpose — keep the worker boundary tight.
 */

const ARGS_EXCERPT_LEN_CAP = 256;

const REDACT_KEYS = new Set<string>([
  'password',
  'token',
  'api_key',
  'apikey',
  'secret',
  'auth',
  'authorization',
  'access_key',
  'access_token',
  'refresh_token',
  'private_key',
]);

function isSecretKey(key: string): boolean {
  const lower = key.toLowerCase();
  if (REDACT_KEYS.has(lower)) return true;
  for (const suffix of REDACT_KEYS) {
    if (lower.endsWith(`_${suffix}`)) return true;
  }
  return false;
}

/** Truncate by Unicode code point so surrogate pairs aren't sliced. */
export function clip(s: string): string {
  if (s.length <= ARGS_EXCERPT_LEN_CAP) return s;
  return `${[...s].slice(0, ARGS_EXCERPT_LEN_CAP).join('')}…`;
}

/**
 * Walk the value tree, redacting secret-keyed string values and clipping
 * long strings. Returns a brand-new tree; the input is never mutated.
 */
export function redact(value: unknown): unknown {
  if (typeof value === 'string') return clip(value);
  if (Array.isArray(value)) return value.map(redact);
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([k, v]) => [k, isSecretKey(k) ? '<redacted>' : redact(v)]),
    );
  }
  return value;
}
