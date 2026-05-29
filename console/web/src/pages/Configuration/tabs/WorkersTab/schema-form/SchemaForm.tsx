/**
 * Top-level entry into the schema-driven form. Owns the in-form state
 * (the working draft) and forwards every field-level change up to the
 * parent via `onChange`. Field components recurse through `FieldDispatch`,
 * which resolves `$ref`s and picks the right primitive.
 *
 * The form deliberately does NOT manage its own dirty/save UX — that's
 * the job of `SaveBar` and the `WorkerEditor` shell. Keeping `SchemaForm`
 * stateless w.r.t. dirty tracking means the same form can be reset to
 * the loaded value (by passing a new `value` prop) without internal
 * resets to clear.
 *
 * Server-side validation errors are passed in as `errors`, a map keyed by
 * JSON Pointer. The dispatcher matches the path it was rendered at against
 * this map and surfaces any matching message on the leaf field.
 */

import type { JsonSchema, JsonValue } from '../api'
import { FieldDispatch } from './FieldDispatch'

export interface SchemaFormProps {
  schema: JsonSchema
  value: JsonValue
  onChange: (next: JsonValue) => void
  /**
   * Map of JSON-Pointer → human-readable error message produced by the
   * last `configuration::set` attempt. Cleared by the parent when the
   * form is edited (so an error doesn't persist past the fix).
   */
  errors?: ReadonlyMap<string, string>
}

export function SchemaForm({
  schema,
  value,
  onChange,
  errors,
}: SchemaFormProps) {
  return (
    <form
      onSubmit={(e) => e.preventDefault()}
      className="space-y-6"
      aria-label="configuration form"
    >
      <FieldDispatch
        label={(schema.title as string) || 'value'}
        schema={schema}
        value={value}
        onChange={onChange}
        rootSchema={schema}
        path={[]}
        errors={errors}
      />
    </form>
  )
}
