import { cn } from '@/lib/utils'
import type { JsonSchema, JsonValue } from '../api'
import { wt } from '../typography'
import { FieldDispatch, type FieldProps } from './FieldDispatch'
import { FieldHelp } from './FieldShell'
import { pathToDomId } from './path'

/**
 * Renders `type: object` schemas with a `properties` map as a titled
 * section followed by an indented stack of child fields.
 *
 * Layout decisions:
 *
 * - The label is rendered as a small section eyebrow rather than the
 *   compact field label primitives use, so deep object trees read as a
 *   document outline instead of a flat list of inputs.
 * - The left rule (`border-l border-rule pl-4`) gives every level a
 *   visual indent without nudging the form right by a full gutter,
 *   which would push deep schemas off-screen on narrow viewports.
 * - Property order honors `schema.properties` insertion order (which
 *   `schemars` preserves from the Rust struct definition).
 * - Required children get a `[required]` badge through `FieldDispatch`
 *   so the chrome is consistent with primitive fields elsewhere.
 *
 * Root-level objects (`path.length === 0`) skip the indent + label
 * because the editor shell already shows the configuration name in its
 * header — duplicating it inside the form would just add noise.
 */
export function ObjectSection(props: FieldProps) {
  const {
    label,
    schema,
    value,
    onChange,
    rootSchema,
    path,
    errors,
    hideLabel,
  } = props
  const properties =
    schema.properties &&
    typeof schema.properties === 'object' &&
    !Array.isArray(schema.properties)
      ? (schema.properties as Record<string, JsonSchema>)
      : {}
  const required = Array.isArray(schema.required)
    ? new Set(schema.required.filter((r): r is string => typeof r === 'string'))
    : new Set<string>()

  const obj = isPlainObject(value) ? value : {}
  const isRoot = path.length === 0
  const description =
    typeof schema.description === 'string' ? schema.description : undefined
  const title =
    typeof schema.title === 'string' && schema.title.length > 0
      ? schema.title
      : label

  const children = Object.entries(properties).map(([key, childSchema]) => (
    <FieldDispatch
      key={key}
      label={key}
      schema={childSchema}
      value={obj[key]}
      onChange={(next) => onChange({ ...obj, [key]: next })}
      rootSchema={rootSchema}
      required={required.has(key)}
      path={[...path, key]}
      errors={errors}
    />
  ))

  // Root forms and label-suppressed renders skip the section header so
  // we don't double up identity. Dictionary entries pass `hideLabel`
  // because the entry itself already carries the "this is a value"
  // signal via its key/header chrome.
  if (isRoot || hideLabel) {
    return (
      <div className="space-y-5">
        {description && !hideLabel ? (
          <p className={cn(wt.bodySm, 'text-ink-faint')}>{description}</p>
        ) : null}
        {children}
      </div>
    )
  }

  return (
    <section id={pathToDomId(path)} className="space-y-2 scroll-mt-24">
      <header className="flex items-baseline gap-2">
        <h3 className={cn(wt.body, 'text-ink lowercase tracking-[-0.01em]')}>
          {title}
        </h3>
        {description ? <FieldHelp description={description} /> : null}
      </header>
      <div className="border-l border-rule pl-4 space-y-5">{children}</div>
    </section>
  )
}

function isPlainObject(
  value: JsonValue | undefined,
): value is { [key: string]: JsonValue } {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
