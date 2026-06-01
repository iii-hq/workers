/**
 * Single dispatch point that picks the right field component for a
 * (schema, value) pair. Every recursive call from `ObjectSection`,
 * `DictionaryField`, `ArrayField`, etc. funnels through here so the
 * type → component mapping lives in exactly one place.
 *
 * Dispatch order is deliberately precedence-ordered, not type-driven:
 * an `enum` always wins regardless of `type`, a `oneOf` wins next, and
 * a nullable union is handled before the underlying type is rendered.
 * Otherwise the standard JSON Schema type keyword decides.
 */

import { cn } from '@/lib/utils'
import type { JsonSchema, JsonValue } from '../api'
import { wt } from '../typography'
import { ArrayField } from './ArrayField'
import { BooleanField } from './BooleanField'
import { DictionaryField } from './DictionaryField'
import { EnumField } from './EnumField'
import { NullableField } from './NullableField'
import { NumberField } from './NumberField'
import { ObjectSection } from './ObjectSection'
import { OneOfField } from './OneOfField'
import { isNullable, resolveSchema, schemaTypes } from './ref-resolver'
import { StringField } from './StringField'

export interface FieldProps {
  /**
   * Display label for the field. The dispatcher itself does not render
   * the label — every concrete field component is responsible for its
   * own labelling because the label placement differs (inline for
   * primitives, header for sections, row for dictionary entries).
   */
  label: string
  schema: JsonSchema
  value: JsonValue | undefined
  onChange: (next: JsonValue) => void
  /**
   * Full root schema, threaded through every recursive call so `$ref`
   * lookups can resolve back to `#/definitions/…`.
   */
  rootSchema: JsonSchema
  /**
   * True when the field is required by its parent object. Surfaces as a
   * small `[required]` badge so operators know the entry can't be unset.
   */
  required?: boolean
  /**
   * Field path from the root value. Threaded through so leaf fields can
   * look up their own server-side error message in the `errors` map and
   * so dictionary / array entries can build stable React keys.
   */
  path: ReadonlyArray<string | number>
  /**
   * Map of JSON-Pointer → human-readable error from the last
   * `configuration::set` attempt. Passed through every level so the
   * leaf that owns a given path can render the matching message.
   */
  errors?: ReadonlyMap<string, string>
  /**
   * Suppress the field's own label/header chrome when the parent already
   * provides identity (e.g. a dictionary row whose key input IS the label,
   * or an array row whose position is the label). Honored by
   * `ObjectSection` and `FieldShell` (and forwarded by the primitive
   * fields); primitives without a parent-supplied label fall back to their
   * default rendering.
   */
  hideLabel?: boolean
}

export function FieldDispatch(props: FieldProps) {
  const resolved = resolveSchema(props.schema, { root: props.rootSchema })
  const next: FieldProps = { ...props, schema: resolved }

  if (Array.isArray(resolved.enum)) {
    return <EnumField {...next} />
  }
  if (Array.isArray(resolved.oneOf) || Array.isArray(resolved.anyOf)) {
    return <OneOfField {...next} />
  }
  if (isNullable(resolved)) {
    return <NullableField {...next} />
  }

  const types = schemaTypes(resolved)
  const primary = types[0]

  if (primary === 'object') {
    if (resolved.properties && typeof resolved.properties === 'object') {
      return <ObjectSection {...next} />
    }
    if (
      resolved.additionalProperties &&
      resolved.additionalProperties !== false
    ) {
      return <DictionaryField {...next} />
    }
    return <UnknownField {...next} />
  }
  if (primary === 'array') return <ArrayField {...next} />
  if (primary === 'string') return <StringField {...next} />
  if (primary === 'integer' || primary === 'number') {
    return <NumberField {...next} />
  }
  if (primary === 'boolean') return <BooleanField {...next} />

  // Unknown / unsupported — render a read-only JSON preview so the
  // operator at least sees the stored value.
  return <UnknownField {...next} />
}

/**
 * Inline last-resort renderer for schemas the dispatcher can't classify
 * (e.g. `{ type: "object" }` with no `properties` or `additionalProperties`,
 * or a schema with no `type` and no `enum`/`oneOf`). Shown as a
 * read-only JSON block with a short note so the operator knows why
 * editing is disabled.
 */
function UnknownField({ label, value }: FieldProps) {
  return (
    <div className="space-y-1">
      <div className="flex items-baseline gap-2">
        <span className={cn(wt.bodySm, 'text-ink lowercase')}>{label}</span>
        <span className={cn(wt.micro, 'text-ink-ghost')}>
          [unsupported schema · read-only]
        </span>
      </div>
      <pre
        className={cn(
          wt.bodySm,
          'text-ink-faint bg-panel p-2 border border-rule overflow-x-auto whitespace-pre-wrap',
        )}
      >
        {JSON.stringify(value, null, 2)}
      </pre>
    </div>
  )
}
