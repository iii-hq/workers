/**
 * An input schema as form fields, and form text back into a payload. Pure:
 * the Test tab builds its form from these and keeps the payload as the one
 * source of truth, so the CLI line and the raw JSON never drift from it.
 */

export type FieldKind = 'string' | 'enum' | 'number' | 'boolean' | 'string-list' | 'json'

export interface FormField {
  name: string
  label: string
  kind: FieldKind
  required: boolean
  description?: string
  enumValues?: string[]
  hint?: string
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function isStringEnum(schema: Record<string, unknown>): boolean {
  const values = schema.enum
  return Array.isArray(values) && values.length > 0 && values.every((v) => typeof v === 'string')
}

/** `type` may be a union (`["string", "null"]`): its first non-null member counts. */
function primaryType(schema: Record<string, unknown>): unknown {
  const t = schema.type
  return Array.isArray(t) ? t.find((x) => x !== 'null') : t
}

function kindOf(schema: Record<string, unknown>): FieldKind {
  if (schema.oneOf !== undefined || schema.anyOf !== undefined || schema.allOf !== undefined) return 'json'
  if (isStringEnum(schema)) return 'enum'
  const type = primaryType(schema)
  if (type === 'string') return 'string'
  if (type === 'number' || type === 'integer') return 'number'
  if (type === 'boolean') return 'boolean'
  const items = schema.items
  // One value per line can only say strings; numbers and booleans would read back as text.
  if (type === 'array' && isRecord(items) && (items.type === 'string' || isStringEnum(items))) return 'string-list'
  return 'json'
}

function field(name: string, prop: unknown, required: boolean): FormField {
  const schema = isRecord(prop) ? prop : {}
  const kind = kindOf(schema)
  const out: FormField = { name, label: typeof schema.title === 'string' ? schema.title : name, kind, required }
  if (typeof schema.description === 'string') out.description = schema.description
  if (kind === 'enum') out.enumValues = schema.enum as string[]
  const hint = [
    schema.minimum !== undefined && `min ${schema.minimum}`,
    schema.maximum !== undefined && `max ${schema.maximum}`,
    schema.default !== undefined &&
      `default ${typeof schema.default === 'string' ? schema.default : JSON.stringify(schema.default)}`,
  ].filter(Boolean)
  if (hint.length > 0) out.hint = hint.join(' · ')
  return out
}

/** Required fields in the schema's `required` order, then the rest in `properties` order. */
export function fieldsFromSchema(schema: unknown): { required: FormField[]; optional: FormField[] } {
  if (!isRecord(schema) || !isRecord(schema.properties)) return { required: [], optional: [] }
  const props = schema.properties
  const names = Array.isArray(schema.required)
    ? schema.required.filter((k): k is string => typeof k === 'string' && Object.hasOwn(props, k))
    : []
  return {
    required: names.map((name) => field(name, props[name], true)),
    optional: Object.keys(props)
      .filter((name) => !names.includes(name))
      .map((name) => field(name, props[name], false)),
  }
}

/** The payload value for what was typed, or `undefined` when the field is blank. */
function typedValue(field: FormField, raw: string | boolean | null): unknown {
  if (raw === null || typeof raw === 'boolean') return raw ?? undefined
  if (field.kind === 'string-list') {
    const lines = raw
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean)
    return lines.length > 0 ? lines : undefined
  }
  if (raw.trim() === '') return undefined
  if (field.kind === 'number') {
    // Text that is not a number goes as typed: the server's own error says what it wants.
    const n = Number(raw)
    return Number.isFinite(n) ? n : raw
  }
  // A `json` field that does not parse throws; the panel keeps the text as a draft.
  if (field.kind === 'json') return JSON.parse(raw)
  return raw
}

/** A new payload with the field set from what was typed; a blank field removes its key. */
export function setField(
  payload: Record<string, unknown>,
  field: FormField,
  raw: string | boolean | null,
): Record<string, unknown> {
  const value = typedValue(field, raw)
  const { [field.name]: _old, ...rest } = payload
  return value === undefined ? rest : { ...rest, [field.name]: value }
}

/** The field's payload value as the text its input shows. */
export function fieldText(payload: Record<string, unknown>, field: FormField): string {
  const value = payload[field.name]
  if (value === undefined) return ''
  if (field.kind === 'json') return JSON.stringify(value, null, 2)
  if (value === null) return ''
  if (field.kind === 'string-list' && Array.isArray(value)) return value.join('\n')
  return typeof value === 'string' ? value : JSON.stringify(value)
}

/** The schema's required keys the payload leaves absent or empty. */
export function missingRequired(schema: unknown, payload: Record<string, unknown>): string[] {
  if (!isRecord(schema) || !isRecord(payload) || !Array.isArray(schema.required)) return []
  return schema.required.filter(
    (k): k is string => typeof k === 'string' && (payload[k] === undefined || payload[k] === ''),
  )
}

/** The disclosure that holds the optional fields. */
export function optionalLabel(required: number, optional: number): string {
  return `${optional} ${required > 0 ? 'more ' : ''}optional input${optional === 1 ? '' : 's'}`
}
