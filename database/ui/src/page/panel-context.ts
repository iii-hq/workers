import type { JsonValue } from '@iii-dev/console-ui'

/** Opens the page with a table pre-selected — the palette source, or any
    other worker's "inspect this table" affordance. */
export interface DatabasePanelContext {
  db?: string
  table: string
}

function asRecord(value: JsonValue): Record<string, JsonValue> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value
    : null
}

export function parseDatabasePanelContext(
  value: JsonValue,
): DatabasePanelContext | null {
  const record = asRecord(value)
  if (!record || typeof record.table !== 'string' || record.table === '')
    return null
  return {
    table: record.table,
    db: typeof record.db === 'string' ? record.db : undefined,
  }
}
