/**
 * Recursive argument redaction for denial envelopes. Pure and immutable:
 * never mutates the input. Lives inside the approval-gate worker (not in
 * a shared utility) on purpose — keep the worker boundary tight.
 */

const ARGS_EXCERPT_LEN_CAP = 256;

/**
 * Hard ceiling on how deep we recurse. Args are caller-influenced, so an
 * adversarial payload (deeply nested or self-referential) must not overflow
 * the stack — past this depth the subtree collapses to a sentinel.
 */
const MAX_REDACT_DEPTH = 64;
const MAX_DEPTH_SENTINEL = '<max-depth>';

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

/**
 * Truncate by Unicode code point so surrogate pairs aren't sliced. The cap
 * is measured in code points too — using UTF-16 `.length` here would falsely
 * clip (and ellipsize) astral-heavy strings that are within the cap.
 */
export function clip(s: string): string {
  const points = [...s];
  if (points.length <= ARGS_EXCERPT_LEN_CAP) return s;
  return `${points.slice(0, ARGS_EXCERPT_LEN_CAP).join('')}…`;
}

/**
 * Walk the value tree, redacting secret-keyed string values and clipping
 * long strings. Returns a brand-new tree; the input is never mutated.
 * Recursion is depth-capped (see MAX_REDACT_DEPTH) so hostile input can't
 * overflow the stack; this also defuses circular references.
 */
export function redact(value: unknown): unknown {
  return redactAt(value, 0);
}

function redactAt(value: unknown, depth: number): unknown {
  if (typeof value === 'string') return clip(value);
  if (value === null || typeof value !== 'object') return value;
  if (depth >= MAX_REDACT_DEPTH) return MAX_DEPTH_SENTINEL;
  if (Array.isArray(value)) return value.map((v) => redactAt(v, depth + 1));
  return Object.fromEntries(
    Object.entries(value).map(([k, v]) => [
      k,
      isSecretKey(k) ? '<redacted>' : redactAt(v, depth + 1),
    ]),
  );
}
