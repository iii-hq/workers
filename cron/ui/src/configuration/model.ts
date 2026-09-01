import type { JsonValue } from '@iii-dev/console-ui'

export type JsonObject = { [key: string]: JsonValue }

export const DEFAULT_ADAPTER_NAME = 'local'

export interface CronConfigurationModel {
  value: JsonObject
  hasOpaqueRoot: boolean
  adapter: JsonObject
  adapterName: string
  adapterConfig: JsonObject | null
  redisUrl: string
  hasOpaqueAdapter: boolean
  hasOpaqueAdapterConfig: boolean
}

export function isObject(value: JsonValue | undefined): value is JsonObject {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function asString(value: JsonValue | undefined): string {
  return typeof value === 'string' ? value : ''
}

/**
 * Read the small Cron configuration without normalizing the stored value.
 * Rendering defaults must not materialize them or discard future fields.
 */
export function readCronConfiguration(value: JsonValue): CronConfigurationModel {
  const hasOpaqueRoot = !isObject(value)
  const root = isObject(value) ? { ...value } : {}
  const hasOpaqueAdapter = root.adapter !== undefined && !isObject(root.adapter)
  const adapter = isObject(root.adapter) ? { ...root.adapter } : {}
  const adapterName = asString(adapter.name) || DEFAULT_ADAPTER_NAME
  const hasOpaqueAdapterConfig = adapter.config !== undefined && !isObject(adapter.config)
  const adapterConfig = isObject(adapter.config) ? { ...adapter.config } : null

  return {
    value: root,
    hasOpaqueRoot,
    adapter,
    adapterName,
    adapterConfig,
    redisUrl: asString(adapterConfig?.redis_url),
    hasOpaqueAdapter,
    hasOpaqueAdapterConfig,
  }
}

/** Change only the adapter discriminator. Dormant config and future fields survive. */
export function withAdapterName(value: JsonValue, name: string): JsonValue {
  const model = readCronConfiguration(value)
  if (model.hasOpaqueRoot) return value
  if (name === model.adapterName) return model.value
  if (model.hasOpaqueAdapter) return model.value

  return {
    ...model.value,
    adapter: {
      ...model.adapter,
      name,
    },
  }
}

/** Replace an opaque whole-adapter value only after an explicit user action. */
export function withAdapterValue(value: JsonValue, adapterValue: JsonValue): JsonValue {
  const model = readCronConfiguration(value)
  if (model.hasOpaqueRoot) return value
  return { ...model.value, adapter: adapterValue }
}

/** Edit or clear the adapter's opaque config while preserving its siblings. */
export function withAdapterConfigValue(value: JsonValue, configValue: JsonValue | undefined): JsonValue {
  const model = readCronConfiguration(value)
  if (model.hasOpaqueRoot) return value
  if (model.hasOpaqueAdapter) return model.value

  const adapter: JsonObject = { ...model.adapter, name: model.adapterName }
  if (configValue === undefined) delete adapter.config
  else adapter.config = configValue
  return { ...model.value, adapter }
}

/** Change only Redis' known URL field while retaining every sibling field. */
export function withRedisUrl(value: JsonValue, redisUrl: string): JsonValue {
  const model = readCronConfiguration(value)
  if (model.hasOpaqueRoot) return value
  if (model.hasOpaqueAdapter || model.hasOpaqueAdapterConfig) return model.value
  if (redisUrl === model.redisUrl) return model.value

  const config = { ...(model.adapterConfig ?? {}) }
  if (redisUrl === '') delete config.redis_url
  else config.redis_url = redisUrl

  const adapter: JsonObject = { ...model.adapter, name: model.adapterName }
  if (Object.keys(config).length > 0) adapter.config = config
  else delete adapter.config

  return { ...model.value, adapter }
}
