import type { JsonSchema } from '../api'

/**
 * True when `schema` is a renderable schema object — a non-null, non-array
 * object. The `configuration` worker can return an entry whose schema is
 * `null` (a worker that registered a config value but no JSON schema). The
 * form dereferences `schema.title` and recurses its fields, so a null/array/
 * primitive schema crashes it; callers gate on this and show an empty state.
 */
export function isObjectSchema(schema: unknown): schema is JsonSchema {
  return typeof schema === 'object' && schema !== null && !Array.isArray(schema)
}
