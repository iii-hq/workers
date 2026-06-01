/**
 * Thin client for the built-in `configuration` worker
 * (configuration::register/get/set/schema/list).
 *
 * The harness owns one configuration entry — id `harness` — that holds the
 * provider registry (api keys + per-provider settings) and the permissions
 * block. This module is the single call site for talking to the worker so
 * the registry/resolve code never sees the raw `iii.trigger` surface.
 */

import type { ISdk } from './iii.js';
import { logger } from './otel.js';

/** A draft-07 JSON Schema object, kept loose on purpose. */
export type JsonSchema = Record<string, unknown>;

/** Any JSON value a configuration entry can hold. */
export type JsonValue =
  | null
  | string
  | number
  | boolean
  | JsonValue[]
  | { [key: string]: JsonValue };

const CONFIG_TIMEOUT_MS = 5_000;

export interface RegisterInput {
  id: string;
  name: string;
  description: string;
  schema: JsonSchema;
  initial_value?: JsonValue;
}

/**
 * Declare (or re-declare) a configuration entry. Idempotent: re-registering
 * replaces the schema + metadata and preserves any existing value.
 */
export async function configurationRegister(iii: ISdk, input: RegisterInput): Promise<void> {
  await iii.trigger({
    function_id: 'configuration::register',
    payload: input,
    timeoutMs: CONFIG_TIMEOUT_MS,
  });
}

/**
 * Read one entry's value. Returns `null` when the entry is missing, has no
 * value yet, or the worker is unreachable — callers always have a sane
 * fallback so a missing `configuration` worker never tanks a request.
 *
 * `raw: false` (the default) expands `${VAR:default}` placeholders against
 * live env; pass `raw: true` to read the stored template form verbatim.
 */
export async function configurationGet(
  iii: ISdk,
  id: string,
  opts: { raw?: boolean } = {},
): Promise<JsonValue | null> {
  try {
    const res = await iii.trigger<unknown, { id?: string; value?: JsonValue }>({
      function_id: 'configuration::get',
      payload: { id, raw: opts.raw ?? false },
      timeoutMs: CONFIG_TIMEOUT_MS,
    });
    return res && typeof res === 'object' ? (res.value ?? null) : null;
  } catch (err) {
    logger.warn('configuration::get failed', { id, err: String(err) });
    return null;
  }
}

/** Replace an entry's value. Validates against the registered schema. */
export async function configurationSet(iii: ISdk, id: string, value: JsonValue): Promise<void> {
  await iii.trigger({
    function_id: 'configuration::set',
    payload: { id, value },
    timeoutMs: CONFIG_TIMEOUT_MS,
  });
}
