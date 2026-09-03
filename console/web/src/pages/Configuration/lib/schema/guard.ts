import type { JsonSchema } from '../../tabs/WorkersTab/api'

/**
 * True when `schema` is an object the host validator can inspect. The
 * `configuration` worker may return `null` for an entry without a JSON
 * Schema; injected forms can still render that entry, but the host skips
 * schema-based draft validation.
 */
export function isObjectSchema(schema: unknown): schema is JsonSchema {
  return typeof schema === 'object' && schema !== null && !Array.isArray(schema)
}
