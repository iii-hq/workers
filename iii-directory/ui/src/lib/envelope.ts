/** `{ content: [...], details }` harness result envelope → details.
 * Idempotent: an already-flat payload passes through unchanged. */
export function unwrapEnvelope(value: unknown): unknown {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return value
  const obj = value as Record<string, unknown>
  if (Array.isArray(obj.content) && 'details' in obj) return obj.details
  return value
}

/** Error-shaped output (engine `{ error: ... }` wrapper). The renderer
 * returns null for these so the console's default error cards apply. */
export function isErrorOutput(value: unknown): boolean {
  return (
    !!value &&
    typeof value === 'object' &&
    !Array.isArray(value) &&
    'error' in (value as Record<string, unknown>)
  )
}
