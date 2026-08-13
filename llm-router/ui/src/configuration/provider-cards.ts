/**
 * Build the provider card list from both the registered schema and the saved
 * configuration. A fresh installation has a null configuration value, while
 * its schema already contains the providers that operators can configure.
 */

type JsonObject = Record<string, unknown>

function asObject(value: unknown): JsonObject {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as JsonObject)
    : {}
}

function schemaProviderIds(
  schema: Record<string, unknown> | null,
): string[] {
  const properties = asObject(schema?.properties)
  const providers = asObject(properties.providers)
  return Object.keys(asObject(providers.properties))
}

export function providerCardIds(
  schema: Record<string, unknown> | null,
  value: unknown,
): string[] {
  const configuredProviders = asObject(asObject(value).providers)
  return [
    ...new Set([
      ...schemaProviderIds(schema),
      ...Object.keys(configuredProviders),
    ]),
  ]
}
