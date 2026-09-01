/**
 * Build the provider card list from both the registered schema and the saved
 * configuration. A fresh installation has a null configuration value, while
 * its schema already contains the providers that operators can configure.
 */

type JsonObject = Record<string, unknown>

export type ProviderFieldKind = 'string' | 'number' | 'integer' | 'boolean' | 'structured'

export interface ProviderFieldDefinition {
  key: string
  label: string
  description?: string
  kind: ProviderFieldKind
  writeOnly: boolean
  required: boolean
  enumValues?: string[]
  defaultValue?: unknown
}

function asObject(value: unknown): JsonObject {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? (value as JsonObject) : {}
}

function schemaProviderIds(schema: Record<string, unknown> | null): string[] {
  const properties = asObject(schema?.properties)
  const providers = asObject(properties.providers)
  return Object.keys(asObject(providers.properties))
}

export function providerSchema(schema: Record<string, unknown> | null, providerId: string): JsonObject {
  const properties = asObject(schema?.properties)
  const providers = asObject(properties.providers)
  return asObject(asObject(providers.properties)[providerId])
}

function fieldKind(definition: JsonObject, value: unknown): ProviderFieldKind {
  const declared = Array.isArray(definition.type)
    ? definition.type.find((candidate) => candidate !== 'null')
    : definition.type
  if (declared === 'string') return 'string'
  if (declared === 'number') return 'number'
  if (declared === 'integer') return 'integer'
  if (declared === 'boolean') return 'boolean'
  if (declared === 'object' || declared === 'array') return 'structured'
  if (typeof value === 'string' || value == null) return 'string'
  if (typeof value === 'number') return 'number'
  if (typeof value === 'boolean') return 'boolean'
  return 'structured'
}

function fieldLabel(key: string): string {
  const words = key.replace(/[_-]+/g, ' ').trim()
  return words ? words[0].toUpperCase() + words.slice(1) : key
}

/**
 * Deliberate flat editor contract for provider-owned config. Schema-declared
 * scalar fields are editable; existing unknown scalar fields remain editable;
 * object and array values are surfaced as preserved, never serialized into a
 * raw JSON control.
 */
export function providerFieldDefinitions(
  schema: Record<string, unknown> | null,
  value: unknown,
  providerId: string,
): ProviderFieldDefinition[] {
  const provider = providerSchema(schema, providerId)
  const properties = asObject(provider.properties)
  const slice = asObject(asObject(asObject(value).providers)[providerId])
  const keys = [...new Set([...Object.keys(properties), ...Object.keys(slice)])]
  const required = new Set(
    Array.isArray(provider.required)
      ? provider.required.filter((candidate): candidate is string => typeof candidate === 'string')
      : [],
  )

  return keys.map((key) => {
    const definition = asObject(properties[key])
    const enumValues = Array.isArray(definition.enum)
      ? definition.enum.filter((candidate): candidate is string => typeof candidate === 'string')
      : undefined
    return {
      key,
      label: typeof definition.title === 'string' ? definition.title : fieldLabel(key),
      description: typeof definition.description === 'string' ? definition.description : undefined,
      kind: fieldKind(definition, slice[key]),
      writeOnly: definition.writeOnly === true || /(?:key|token|secret|password)/i.test(key),
      required: required.has(key),
      ...(enumValues && enumValues.length > 0 ? { enumValues } : {}),
      ...('default' in definition ? { defaultValue: definition.default } : {}),
    }
  })
}

export function providerCardIds(schema: Record<string, unknown> | null, value: unknown): string[] {
  const configuredProviders = asObject(asObject(value).providers)
  return [...new Set([...schemaProviderIds(schema), ...Object.keys(configuredProviders)])]
}
