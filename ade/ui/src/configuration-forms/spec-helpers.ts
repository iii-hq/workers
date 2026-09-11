import type { JsonValue } from '@iii-dev/console-ui'
import type {
  Choice,
  ConfigPath,
  DynamicMapFieldSpec,
  FilterListFieldSpec,
  FormFieldSpec,
  NumberFieldSpec,
  ObjectCollectionFieldSpec,
  ObjectFieldSpec,
  ObjectMapFieldSpec,
  PermissionRulesFieldSpec,
  SelectFieldSpec,
  StringListFieldSpec,
  StructuredValueFieldSpec,
  SwitchFieldSpec,
  TextFieldSpec,
  VariantFieldSpec,
  VariantOptionSpec,
} from './types'

export const p = (value: string): ConfigPath => (value === '' ? [] : value.split('.'))

export const text = (
  path: string,
  label: string,
  description?: string,
  options: Partial<Omit<TextFieldSpec, 'kind' | 'path' | 'label' | 'description'>> = {},
): TextFieldSpec => ({ kind: 'text', path: p(path), label, description, ...options })

export const password = (
  path: string,
  label: string,
  description?: string,
  options: Partial<Omit<TextFieldSpec, 'kind' | 'path' | 'label' | 'description'>> = {},
): TextFieldSpec => ({ kind: 'password', path: p(path), label, description, ...options })

export const number = (
  path: string,
  label: string,
  description?: string,
  options: Partial<Omit<NumberFieldSpec, 'kind' | 'path' | 'label' | 'description'>> = {},
): NumberFieldSpec => ({ kind: 'number', path: p(path), label, description, ...options })

export const select = (
  path: string,
  label: string,
  optionsList: readonly Choice[],
  description?: string,
  options: Partial<Omit<SelectFieldSpec, 'kind' | 'path' | 'label' | 'description' | 'options'>> = {},
): SelectFieldSpec => ({
  kind: 'select',
  path: p(path),
  label,
  description,
  options: optionsList,
  ...options,
})

export const toggle = (
  path: string,
  label: string,
  description?: string,
  options: Partial<Omit<SwitchFieldSpec, 'kind' | 'path' | 'label' | 'description'>> = {},
): SwitchFieldSpec => ({ kind: 'switch', path: p(path), label, description, ...options })

export const stringList = (
  path: string,
  label: string,
  description?: string,
  options: Partial<Omit<StringListFieldSpec, 'kind' | 'path' | 'label' | 'description'>> = {},
): StringListFieldSpec => ({
  kind: 'string-list',
  path: p(path),
  label,
  description,
  ...options,
})

export const dynamicMap = (
  path: string,
  label: string,
  description?: string,
  options: Partial<Omit<DynamicMapFieldSpec, 'kind' | 'path' | 'label' | 'description'>> = {},
): DynamicMapFieldSpec => ({
  kind: 'dynamic-map',
  path: p(path),
  label,
  description,
  ...options,
})

export const filterList = (path: string, label: string, description?: string): FilterListFieldSpec => ({
  kind: 'filter-list',
  path: p(path),
  label,
  description,
})

export const permissionRules = (
  path: string,
  label: string,
  description?: string,
  options: Partial<Omit<PermissionRulesFieldSpec, 'kind' | 'path' | 'label' | 'description'>> = {},
): PermissionRulesFieldSpec => ({
  kind: 'permission-rules',
  path: p(path),
  label,
  description,
  ...options,
})

export const structuredValue = (
  path: string,
  label: string,
  description?: string,
  options: Partial<Omit<StructuredValueFieldSpec, 'kind' | 'path' | 'label' | 'description'>> = {},
): StructuredValueFieldSpec => ({
  kind: 'structured-value',
  path: p(path),
  label,
  description,
  ...options,
})

export const object = (
  path: string,
  label: string,
  fields: readonly FormFieldSpec[],
  description?: string,
  options: Partial<Omit<ObjectFieldSpec, 'kind' | 'path' | 'label' | 'description' | 'fields'>> = {},
): ObjectFieldSpec => ({ kind: 'object', path: p(path), label, description, fields, ...options })

export const variantOption = (
  value: string,
  label: string,
  defaultValue: Readonly<Record<string, JsonValue>>,
  fields: readonly FormFieldSpec[],
  description?: string,
): VariantOptionSpec => ({ value, label, defaultValue, fields, description })

export const variant = (
  path: string,
  label: string,
  optionsList: readonly VariantOptionSpec[],
  description?: string,
  options: Partial<Omit<VariantFieldSpec, 'kind' | 'path' | 'label' | 'description' | 'options'>> = {},
): VariantFieldSpec => ({
  kind: 'variant',
  path: p(path),
  label,
  description,
  options: optionsList,
  ...options,
})

export const objectList = (
  path: string,
  label: string,
  itemLabel: string,
  createItem: Readonly<Record<string, JsonValue>>,
  fields: readonly FormFieldSpec[],
  description?: string,
  options: Partial<
    Omit<ObjectCollectionFieldSpec, 'kind' | 'path' | 'label' | 'description' | 'itemLabel' | 'createItem' | 'fields'>
  > = {},
): ObjectCollectionFieldSpec => ({
  kind: 'object-list',
  path: p(path),
  label,
  description,
  itemLabel,
  createItem,
  fields,
  ...options,
})

export const objectMap = (
  path: string,
  label: string,
  itemLabel: string,
  createValue: Readonly<Record<string, JsonValue>>,
  fields: readonly FormFieldSpec[],
  description?: string,
  options: Partial<
    Omit<ObjectMapFieldSpec, 'kind' | 'path' | 'label' | 'description' | 'itemLabel' | 'createValue' | 'fields'>
  > = {},
): ObjectMapFieldSpec => ({
  kind: 'object-map',
  path: p(path),
  label,
  description,
  itemLabel,
  createValue,
  fields,
  ...options,
})
