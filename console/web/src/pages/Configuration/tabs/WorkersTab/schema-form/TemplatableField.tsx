/**
 * Shared scalar ⇄ env-template wrapper for typed leaf fields (number, boolean).
 *
 * `FieldDispatch` routes by *schema* type, so a number/boolean field can be
 * handed a string value — a `${VAR}` template a human (or another editor) put
 * in the config. The native numeric/boolean controls can't represent that, so
 * without this wrapper the template renders blank and the first edit silently
 * discards it. This wrapper lets the same field hold either form:
 *
 *   - value is a string → TEMPLATE mode → the `${VAR}` pill editor
 *     (`EnvLexicalInput`), identical to how `StringField` handles templates;
 *   - otherwise → SCALAR mode → the field's native control (`renderScalar`).
 *
 * Mode is derived purely from the value type (no local state that could
 * desync from a re-seeded draft). Switching modes just rewrites the value:
 * "use an env variable" sets `''` (a string ⇒ template mode); "use a value"
 * sets `scalarDefault` (a non-string ⇒ scalar mode).
 */

import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'
import type { JsonValue } from '../api'
import { wt } from '../typography'
import { EnvLexicalInput } from './env-lexical/EnvLexicalInput'
import type { FieldProps } from './FieldDispatch'
import { errorForField, FieldShell } from './FieldShell'
import { pathToDomId } from './path'

interface TemplatableFieldProps {
  props: FieldProps
  /** Value installed when leaving template mode (e.g. `null`, `false`). */
  scalarDefault: JsonValue
  /** Renders the native scalar control, given the FieldShell control id. */
  renderScalar: (controlId: string) => ReactNode
}

export function TemplatableField({
  props,
  scalarDefault,
  renderScalar,
}: TemplatableFieldProps) {
  const { label, schema, value, onChange, required, hideLabel } = props
  const description =
    typeof schema.description === 'string' ? schema.description : undefined
  const isTemplate = typeof value === 'string'

  return (
    <FieldShell
      label={label}
      description={description}
      required={required}
      errorMessage={errorForField(props)}
      hideLabel={hideLabel}
      anchorId={pathToDomId(props.path)}
    >
      {(controlId) => (
        <div className="space-y-1.5">
          {isTemplate ? (
            <EnvLexicalInput
              id={controlId}
              value={value}
              onChange={onChange}
              aria-label={label}
            />
          ) : (
            renderScalar(controlId)
          )}
          <button
            type="button"
            onClick={() => onChange(isTemplate ? scalarDefault : '')}
            className={cn(
              wt.micro,
              'text-ink-ghost hover:text-ink transition-colors lowercase',
            )}
          >
            {isTemplate ? 'use a literal value' : 'use an env variable'}
          </button>
        </div>
      )}
    </FieldShell>
  )
}
