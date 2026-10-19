import type { JsonValue } from '@iii-dev/console-ui'

/** Opens the page on a connection, optionally with a table pre-selected —
    the palette source, a `@database(id="…")` mention, or any other worker's
    "inspect this table" affordance. */
export interface DatabasePanelContext {
  db?: string
  table?: string
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
  if (!record) return null
  const table =
    typeof record.table === 'string' && record.table !== ''
      ? record.table
      : undefined
  const db =
    typeof record.db === 'string' && record.db !== '' ? record.db : undefined
  return table || db ? { table, db } : null
}
