import type { IIIClient } from 'iii-sdk';
import { z } from 'zod';
import {
  type Config,
  type ConfigHolder,
  ConfigSchema,
  configId,
  DEFAULT_CONFIG_ID,
  defaultConfig,
  runtimeJsonSchema,
} from './config.js';
import { jsonSchema } from './schema.js';

const CONFIG_FN_ID = 'cursor::on-config-change';
const TIMEOUT_MS = 5_000;
const RETRY_DELAYS_MS = [250, 500, 1_000];

export const ConfigChangeEventSchema = z.object({ id: z.string().optional() }).passthrough();
export const ConfigChangeResponseSchema = z.object({ ok: z.boolean() });

/** Prefer atomic ensure; engines lacking it use warned, non-atomic legacy initialization. */
export async function registerCursorConfig(
  iii: IIIClient,
  initialValue: Config = defaultConfig(),
): Promise<void> {
  const payload: Record<string, unknown> = {
    id: configId(),
    name: 'Cursor',
    description:
      'Cursor provider and agent worker using normal Cursor CLI login for LLM Router and local ACP sessions, plus the optional sdk.v1 Bridge for explicit API-key or cloud sessions.',
    schema: runtimeJsonSchema(),
    metadata: { ui_form: DEFAULT_CONFIG_ID },
    initial_value: initialValue,
  };
  try {
    await triggerWithRetry(iii, 'configuration::ensure', payload);
    return;
  } catch (error) {
    if (!isFunctionNotFound(error)) throw error;
  }
  const id = String(payload.id);
  if (!warnedLegacy.has(id)) {
    warnedLegacy.add(id);
    console.warn(
      `${id}: engine lacks configuration::ensure; using non-atomic legacy initialization; upgrade to >=0.24.1 for concurrent-write safety`,
    );
  }
  let existing: unknown;
  try {
    const response = await triggerWithRetry(iii, 'configuration::get', { id, raw: true });
    if (
      !response ||
      typeof response !== 'object' ||
      Array.isArray(response) ||
      !Object.hasOwn(response, 'value') ||
      !('value' in response) ||
      response.value === undefined
    ) {
      throw new Error('configuration::get returned no `value` field');
    }
    existing = response.value;
  } catch (error) {
    if (!isMissingEntry(error)) throw error;
    existing = null;
  }
  const registration = { ...payload };
  if (existing !== null) delete registration.initial_value;
  await triggerWithRetry(iii, 'configuration::register', registration);
}

const warnedLegacy = new Set<string>();

/** Inspect the structured SDK code, never a word in the message. */
function isFunctionNotFound(error: unknown, functionId = 'configuration::ensure'): boolean {
  return (
    !!error &&
    typeof error === 'object' &&
    'code' in error &&
    error.code === 'function_not_found' &&
    (!('function_id' in error) ||
      error.function_id === undefined ||
      error.function_id === functionId)
  );
}

/** Fetch and validate the applied Cursor configuration; missing or malformed values are errors. */
export async function fetchRuntime(iii: IIIClient): Promise<Config> {
  const response = await triggerWithRetry(iii, 'configuration::get', {
    id: configId(),
    raw: false,
  });
  const parsed = z.object({ value: z.unknown() }).parse(response);
  return ConfigSchema.parse(parsed.value);
}

/** Subscribe before the initial read, serialize reloads, and retain the last valid config on failure. */
export async function bindConfigTrigger(iii: IIIClient, holder: ConfigHolder): Promise<void> {
  let reload = Promise.resolve();
  const refresh = async () => {
    const next = await fetchRuntime(iii);
    holder.current = next;
  };
  const serializedRefresh = async (): Promise<boolean> => {
    let applied = false;
    reload = reload.then(
      async () => {
        await refresh();
        applied = true;
      },
      async () => {
        await refresh();
        applied = true;
      },
    );
    try {
      await reload;
      return applied;
    } catch (error) {
      console.warn(`cursor configuration reload rejected: ${safeError(error)}`);
      return false;
    }
  };

  iii.registerFunction(
    CONFIG_FN_ID,
    async (event: unknown) => {
      ConfigChangeEventSchema.parse(event ?? {});
      return { ok: await serializedRefresh() };
    },
    {
      description: 'Internal Cursor configuration reload hook.',
      request_format: jsonSchema(ConfigChangeEventSchema),
      response_format: jsonSchema(ConfigChangeResponseSchema),
      metadata: { internal: true },
    },
  );
  iii.registerTrigger({
    type: 'configuration',
    function_id: CONFIG_FN_ID,
    config: { configuration_id: configId(), event_types: ['configuration:updated'] },
  });
  await refresh();
}

/** Retry transient configuration RPC failures; a definite missing entry returns immediately. */
async function triggerWithRetry(
  iii: IIIClient,
  functionId: string,
  payload: Record<string, unknown>,
): Promise<unknown> {
  let lastError: unknown;
  for (let attempt = 0; attempt <= RETRY_DELAYS_MS.length; attempt += 1) {
    try {
      return await iii.trigger({
        function_id: functionId,
        namespace: 'default',
        payload,
        timeoutMs: TIMEOUT_MS,
      });
    } catch (error) {
      if (isMissingEntry(error) || isFunctionNotFound(error, functionId)) throw error;
      lastError = error;
      const delay = RETRY_DELAYS_MS[attempt];
      if (delay === undefined) break;
      await new Promise((resolvePromise) => setTimeout(resolvePromise, delay));
    }
  }
  throw lastError;
}

/** Missing entry is distinct from an unavailable configuration service. */
function isMissingEntry(error: unknown): boolean {
  return (
    !!error &&
    typeof error === 'object' &&
    'code' in error &&
    error.code === 'NOT_FOUND' &&
    (!('function_id' in error) ||
      error.function_id === undefined ||
      error.function_id === 'configuration::get')
  );
}

/** Render rejected reloads consistently whether the SDK throws an Error or another value. */
function safeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
