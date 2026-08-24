import type { JsonValue } from '@iii-dev/console-ui'

/** Opens the page with a scope/key pre-selected — the palette source, or
    any other worker's "inspect this key" affordance. */
export interface StatePanelContext {
  scope: string
  key: string
}

function asRecord(value: JsonValue): Record<string, JsonValue> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value
    : null
}

export function parseStatePanelContext(
  value: JsonValue,
): StatePanelContext | null {
  const record = asRecord(value)
  if (!record) return null
  if (typeof record.scope !== 'string' || record.scope === '') return null
  if (typeof record.key !== 'string' || record.key === '') return null
  return { scope: record.scope, key: record.key }
}
