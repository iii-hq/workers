import type { IIIClient } from 'iii-sdk'
import { type Config, type RuntimeConfig, RuntimeConfigSchema, runtimeJsonSchema, toRuntime } from './config.js'

const CONFIG_ID = 'slides'
const CONFIG_FN_ID = 'slides::on-config-change'
const TIMEOUT_MS = 5_000

export type ConfigHolder = { current: Config }

export async function registerSlidesConfig(iii: IIIClient, seed: Config): Promise<void> {
  await iii.trigger({
    function_id: 'configuration::register',
    namespace: 'default',
    payload: {
      id: CONFIG_ID,
      name: 'Slides',
      description:
        'Slides worker: export directory, default theme, the router model used for drafting, and brand accent and footer applied to every deck.',
      schema: runtimeJsonSchema(),
      metadata: { ui_form: CONFIG_ID },
      initial_value: toRuntime(seed),
    },
    timeoutMs: TIMEOUT_MS,
  })
}

export async function fetchRuntime(iii: IIIClient): Promise<RuntimeConfig | null> {
  try {
    const res = await iii.trigger<Record<string, unknown>, { value?: unknown }>({
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

export function bindConfigTrigger(iii: IIIClient, onChange: () => Promise<void>): void {
  iii.registerFunction(
    CONFIG_FN_ID,
    async () => {
      await onChange()
      return null
    },
    {
      description: 'Internal: reload the slides configuration when it changes.',
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
