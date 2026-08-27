import type { IIIClient } from 'iii-sdk'
import { type Config, type RuntimeConfig, RuntimeConfigSchema, runtimeJsonSchema, toRuntime } from './config.js'

export const CONFIG_ID = 'aspire-dashboard'
const CONFIG_FN_ID = 'aspire-dashboard::on-config-change'
const TIMEOUT_MS = 5_000

export type ConfigHolder = { current: Config }

export async function registerAspireDashboardConfig(iii: IIIClient, seed: Config): Promise<void> {
  await iii.trigger({
    function_id: 'configuration::register',
    namespace: 'default',
    payload: {
      id: CONFIG_ID,
      name: 'Aspire Dashboard',
      description:
        'Aspire Dashboard worker: launch command, loopback web UI and OTLP ports, OTLP API-key security, and startup behavior.',
      schema: runtimeJsonSchema(),
      initial_value: toRuntime(seed),
    },
    timeoutMs: TIMEOUT_MS,
  })
}

export async function fetchRuntime(iii: IIIClient): Promise<RuntimeConfig | null> {
  try {
    const res = await iii.trigger<unknown, { value?: unknown }>({
      function_id: 'configuration::get',
      namespace: 'default',
      payload: { id: CONFIG_ID, raw: false },
      timeoutMs: TIMEOUT_MS,
    })
    const value = res && typeof res === 'object' ? res.value : null
    if (value == null) return null
    return RuntimeConfigSchema.parse(value)
  } catch (err) {
    console.warn(`configuration::get failed for ${CONFIG_ID}: ${String(err)}`)
    return null
  }
}

export async function bindConfigTrigger(iii: IIIClient, onChange: () => Promise<void>): Promise<void> {
  await onChange()
  iii.registerFunction(
    CONFIG_FN_ID,
    async () => {
      await onChange()
      return null
    },
    {
      description: 'Internal: reload the Aspire Dashboard worker configuration when it changes.',
      metadata: { internal: true },
      request_format: { type: 'object', properties: {} },
      response_format: { type: 'null' },
    },
  )
  iii.registerTrigger({
    type: 'configuration',
    function_id: CONFIG_FN_ID,
    config: { configuration_id: CONFIG_ID, event_types: ['configuration:updated'] },
  })
}
