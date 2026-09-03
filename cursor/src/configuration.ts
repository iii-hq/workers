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

export async function registerCursorConfig(
  iii: IIIClient,
  initialValue: Config = defaultConfig(),
): Promise<void> {
  await triggerWithRetry(iii, 'configuration::register', {
    id: configId(),
    name: 'Cursor',
    description:
      'Cursor provider and agent worker using normal Cursor CLI login for LLM Router and local ACP sessions, plus the optional sdk.v1 Bridge for explicit API-key or cloud sessions.',
    schema: runtimeJsonSchema(),
    metadata: { ui_form: DEFAULT_CONFIG_ID },
    initial_value: initialValue,
  });
}

export async function fetchRuntime(iii: IIIClient): Promise<Config> {
  const response = await triggerWithRetry(iii, 'configuration::get', {
    id: configId(),
    raw: false,
  });
  const parsed = z.object({ value: z.unknown() }).parse(response);
  return ConfigSchema.parse(parsed.value);
}

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
      lastError = error;
      const delay = RETRY_DELAYS_MS[attempt];
      if (delay === undefined) break;
      await new Promise((resolvePromise) => setTimeout(resolvePromise, delay));
    }
  }
  throw lastError instanceof Error ? lastError : new Error(String(lastError));
}

function safeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
