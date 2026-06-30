/**
 * Typed wrappers over the `configuration` worker bus functions.
 * Follows the same single-call-site pattern as `lib/providers.ts`:
 * every external touchpoint funnels through `getIiiClient()` here so
 * components and hooks never see the raw transport.
 *
 * The editor always reads with `raw: true` so `${VAR:default}` templates
 * are returned verbatim and can be saved back unchanged. The expanded
 * value is a separate read concern (preview hover, etc.) and is fetched
 * via `getConfiguration(id, { raw: false })` on demand.
 */

import { getIiiClient } from '@/lib/iii-client'

/* ------------------------------------------------------------------ */
/*  Shared types                                                       */
/* ------------------------------------------------------------------ */

/**
 * A draft-07 JSON Schema object as exposed by the `configuration` worker.
 * Kept as `Record<string, unknown>` because we do not want to commit to a
 * full schema typing — the form is structural and inspects keys at runtime.
 */
export type JsonSchema = Record<string, unknown>

/**
 * Any JSON value that a configuration can hold. Mirrors the wire format —
 * we never narrow it client-side.
 */
export type JsonValue =
  | null
  | string
  | number
  | boolean
  | JsonValue[]
  | { [key: string]: JsonValue }

/**
 * Schema-side view of a configuration entry as returned by
 * `configuration::list` and `configuration::schema`. Crucially does NOT
 * include the value — operators see the schema first, then load the value
 * separately to keep secrets out of the list response.
 */
export interface ConfigurationSchemaView {
  id: string
  name: string
  description: string
  /** Null when the worker registered a config value but no JSON schema. */
  schema: JsonSchema | null
  metadata?: JsonValue
}

interface ListResponse {
  configurations: ConfigurationSchemaView[]
}

interface GetResponse {
  id: string
  value: JsonValue
}

export interface SetResponse {
  new_value: JsonValue
  old_value: JsonValue | null
}

export interface GetOptions {
  /**
   * When true (the default for the editor), `${VAR:default}` templates are
   * returned verbatim so saves round-trip cleanly. Pass `false` to fetch
   * the env-expanded preview.
   */
  raw?: boolean
}

/* ------------------------------------------------------------------ */
/*  Bus triggers                                                       */
/* ------------------------------------------------------------------ */

export async function listConfigurations(): Promise<ConfigurationSchemaView[]> {
  const client = await getIiiClient()
  const response = await client.trigger<ListResponse>('configuration::list', {})
  return response.configurations ?? []
}

export async function getConfigurationSchema(
  id: string,
): Promise<ConfigurationSchemaView> {
  const client = await getIiiClient()
  return client.trigger<ConfigurationSchemaView>('configuration::schema', {
    id,
  })
}

export async function getConfiguration(
  id: string,
  options: GetOptions = {},
): Promise<JsonValue> {
  const client = await getIiiClient()
  const response = await client.trigger<GetResponse>('configuration::get', {
    id,
    raw: options.raw ?? true,
  })
  return response.value
}

export interface SetConfigurationPayload {
  id: string
  value: JsonValue
}

export async function setConfiguration(
  payload: SetConfigurationPayload,
): Promise<SetResponse> {
  const client = await getIiiClient()
  return client.trigger<SetResponse>(
    'configuration::set',
    payload as unknown as Record<string, unknown>,
  )
}
