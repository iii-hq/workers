import { Input } from '@/components/ui/Input'
import { wt } from '../typography'
import { EnvLexicalInput } from './env-lexical/EnvLexicalInput'
import type { FieldProps } from './FieldDispatch'
import { errorForField, FieldShell } from './FieldShell'

/**
 * Free-form string field. Defaults to the Lexical-based `EnvLexicalInput`
 * so `${VAR}` and `${VAR:default}` templates render as inline pills —
 * the very thing operators most often want when authoring worker
 * configuration values (URLs with env-derived credentials, paths
 * defaulting to a dev value, etc.).
 *
 * Formats where templating doesn't make sense fall back to the plain
 * `Input`:
 *
 *   - `password` — hide the value entirely; pill rendering would leak
 *     structure even when the characters themselves are masked.
 *   - `date`, `date-time`, `time`, `email`, `uri-template`, `regex` —
 *     structured strings whose grammar conflicts with `${…}` (typing a
 *     date and seeing a piece of it become a pill would be jarring).
 *
 * Single-value-enum strings hit `EnumField` via `FieldDispatch`, so we
 * don't need a code branch here.
 */

const NON_TEMPLATED_FORMATS = new Set([
  'password',
  'date',
  'date-time',
  'time',
  'email',
  'uri-template',
  'regex',
])

export function StringField(props: FieldProps) {
  const { label, schema, value, onChange, required, hideLabel } = props
  const current = typeof value === 'string' ? value : ''
  const description =
    typeof schema.description === 'string' ? schema.description : undefined
  const format = typeof schema.format === 'string' ? schema.format : undefined
  const placeholder =
    typeof schema.default === 'string' ? schema.default : undefined
  const useLexical = !format || !NON_TEMPLATED_FORMATS.has(format)

  return (
    <FieldShell
      label={label}
      description={description}
      required={required}
      errorMessage={errorForField(props)}
      hideLabel={hideLabel}
    >
      {(controlId) =>
        useLexical ? (
          <EnvLexicalInput
            id={controlId}
            value={current}
            onChange={onChange}
            placeholder={placeholder}
            aria-label={label}
          />
        ) : (
          <Input
            id={controlId}
            name={label}
            value={current}
            onChange={onChange}
            preserveCase
            type={format === 'password' ? 'password' : 'text'}
            placeholder={placeholder}
            className={wt.control}
          />
        )
      }
    </FieldShell>
  )
}
