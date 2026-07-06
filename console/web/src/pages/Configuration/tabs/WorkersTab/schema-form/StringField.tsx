import { Input } from '@/components/ui/Input'
import { wt } from '../typography'
import { EnvLexicalInput } from './env-lexical/EnvLexicalInput'
import type { FieldProps } from './FieldDispatch'
import { errorForField, FieldShell } from './FieldShell'
import { pathToDomId } from './path'

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
 * `format: "textarea"` renders the same pill editor in multiline mode —
 * for long-form prose values (system prompts, message templates) where a
 * single-line input is unusable. Pills and `${VAR}` templating work
 * exactly as in the single-line editor.
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
  // In textarea mode the schema default is the editor's seed-on-set value
  // (a full prompt), never a placeholder hint — showing it as a placeholder
  // when the field is cleared would spill a multi-KB string out of the box.
  const placeholder =
    format !== 'textarea' && typeof schema.default === 'string'
      ? schema.default
      : undefined
  const useLexical = !format || !NON_TEMPLATED_FORMATS.has(format)

  return (
    <FieldShell
      label={label}
      description={description}
      required={required}
      errorMessage={errorForField(props)}
      hideLabel={hideLabel}
      anchorId={pathToDomId(props.path)}
    >
      {(controlId) =>
        useLexical ? (
          <EnvLexicalInput
            id={controlId}
            value={current}
            onChange={onChange}
            placeholder={placeholder}
            multiline={format === 'textarea'}
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
