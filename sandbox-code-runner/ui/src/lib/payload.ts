/**
 * Wire-payload shape helpers shared by the op cards, the sandbox family and
 * the fleet page. No React, no DOM — safe to import from the headless
 * modules (page/store.ts, page/exec.ts) whose tests run without a renderer.
 */

/** Narrow to a plain object, or `undefined` for anything else (incl. arrays). */
export function asRecord(value: unknown): Record<string, unknown> | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    return undefined
  return value as Record<string, unknown>
}

/** `{ content: [...], details }` harness result envelope → details. */
export function unwrapEnvelope(value: unknown): unknown {
  const obj = asRecord(value)
  if (obj && Array.isArray(obj.content) && 'details' in obj) return obj.details
  return value
}
