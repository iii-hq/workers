/**
 * Transport for the server-side `console` configuration entry (registered by
 * the console worker's Rust side at boot; persisted by the engine's
 * `configuration` worker — one YAML file per id under the engine's ./config).
 *
 * The value is a single JSON object shared by every browser pointing at this
 * engine. `configuration::set` replaces the WHOLE value, so writers must
 * read-modify-write; concurrent tabs are last-write-wins (acceptable for a
 * single-operator dev tool).
 *
 * Only committed-worthy settings live here (port, `data_dir`, traces
 * preferences, injectable-UI toggles). The workspace tab/pane layout is
 * ephemeral and goes through `lib/workspace-layout.ts` instead.
 *
 * When the `configuration` worker is disabled or the entry was never
 * registered, reads resolve to `null` and the UI degrades to in-browser
 * defaults (saved views hidden).
 */

import { resolveConfigurationId } from '@iii-dev/console-ui/configuration'
import { getIiiClient } from '@/lib/iii-client'

export type ConsoleConfigValue = Record<string, unknown>

/** Identify absent configuration services or entries so reads can fall back without noisy warnings. */
function isUnavailable(err: unknown): boolean {
  const message = err instanceof Error ? err.message : String(err)
  return /function[_ ]not[_ ]found|not[_ ]found/i.test(message)
}

/**
 * Read the whole `console` configuration value. `null` means the entry (or
 * the configuration worker itself) is unavailable — callers should hide
 * server-persisted preferences rather than erroring.
 */
export async function fetchConsoleConfigValue(): Promise<ConsoleConfigValue | null> {
  try {
    const client = await getIiiClient()
    const resp = await client.trigger<{ value?: unknown }>(
      'configuration::get',
      { id: await resolveConfigurationId(client, 'console'), raw: true },
    )
    const value = resp?.value
    return value && typeof value === 'object' && !Array.isArray(value)
      ? (value as ConsoleConfigValue)
      : {}
  } catch (err) {
    if (isUnavailable(err)) return null
    throw err instanceof Error ? err : new Error(String(err))
  }
}

/** Replace the whole `console` configuration value (read-modify-write). */
export async function setConsoleConfigValue(
  value: ConsoleConfigValue,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('configuration::set', {
    id: await resolveConfigurationId(client, 'console'),
    value,
  })
}
