import type { JsonValue } from '@iii-dev/console-ui'

export type ConfigPath = readonly string[]

export interface Choice {
  value: string
  label: string
  description?: string
}

interface FieldBase {
  path: ConfigPath
  label: string
  description?: string
  optional?: boolean
}

export interface TextFieldSpec extends FieldBase {
  kind: 'text' | 'password'
  placeholder?: string
}

export interface NumberFieldSpec extends FieldBase {
  kind: 'number'
  min?: number
  max?: number
  step?: number
}

export interface SelectFieldSpec extends FieldBase {
  kind: 'select'
  options: readonly Choice[]
  placeholder?: string
}

export interface SwitchFieldSpec extends FieldBase {
  kind: 'switch'
  defaultValue?: boolean
}

export interface StringListFieldSpec extends FieldBase {
  kind: 'string-list'
  itemLabel?: string
  addLabel?: string
  placeholder?: string
  options?: readonly Choice[]
}

export interface DynamicMapFieldSpec extends FieldBase {
  kind: 'dynamic-map'
  keyLabel?: string
  valueLabel?: string
  addLabel?: string
  secretKeys?: readonly string[]
}

export interface FilterListFieldSpec extends FieldBase {
  kind: 'filter-list'
}

export interface PermissionRulesFieldSpec extends FieldBase {
  kind: 'permission-rules'
  addLabel?: string
}

/** A deliberately structured editor for worker fields whose contract accepts
 * any JSON value. It renders typed key/value and list controls; it never
 * exposes a raw JSON textarea or derives controls from JSON Schema. */
export interface StructuredValueFieldSpec extends FieldBase {
  kind: 'structured-value'
  addLabel?: string
  secretKeys?: readonly string[]
}

export interface ObjectFieldSpec extends FieldBase {
  kind: 'object'
  fields: readonly FormFieldSpec[]
  defaultValue?: Readonly<Record<string, JsonValue>>
  disabledValue?: JsonValue
}

export interface VariantOptionSpec {
  value: string
  label: string
  description?: string
  defaultValue: Readonly<Record<string, JsonValue>>
  fields: readonly FormFieldSpec[]
}

export interface VariantFieldSpec extends FieldBase {
  kind: 'variant'
  discriminator?: string
  contentKey?: string
  options: readonly VariantOptionSpec[]
}

export interface ObjectCollectionFieldSpec extends FieldBase {
  kind: 'object-list'
  itemLabel: string
  addLabel?: string
  emptyLabel?: string
  createItem: Readonly<Record<string, JsonValue>>
  fields: readonly FormFieldSpec[]
  summaryPaths?: readonly ConfigPath[]
}

export interface ObjectMapFieldSpec extends FieldBase {
  kind: 'object-map'
  itemLabel: string
  keyLabel?: string
  addLabel?: string
  emptyLabel?: string
  createValue: Readonly<Record<string, JsonValue>>
  fields: readonly FormFieldSpec[]
  summaryPaths?: readonly ConfigPath[]
}

export type FormFieldSpec =
  | TextFieldSpec
  | NumberFieldSpec
  | SelectFieldSpec
  | SwitchFieldSpec
  | StringListFieldSpec
  | DynamicMapFieldSpec
  | FilterListFieldSpec
  | PermissionRulesFieldSpec
  | StructuredValueFieldSpec
  | ObjectFieldSpec
  | VariantFieldSpec
  | ObjectCollectionFieldSpec
  | ObjectMapFieldSpec

export interface FormSectionSpec {
  title: string
  description?: string
  fields: readonly FormFieldSpec[]
}

export interface WorkerConfigurationSpec {
  id: string
  title: string
  description: string
  /**
   * Older worker releases accepted their settings below a worker-named
   * envelope. When present, the form edits that inner object and migrates it
   * to the current flat shape on the first change.
   */
  legacyWrapper?: string
  sections: readonly FormSectionSpec[]
  /**
   * Explicit contract snapshot. The manifest test compares it with every
   * leaf path reachable through the declarative spec. It is intentionally
   * independent of JSON Schema: schemas validate drafts; they never generate
   * this UI.
   */
  expectedFields: readonly string[]
}

export const choice = (value: string, label?: string): Choice => ({
  value,
  label: label ?? value,
})
