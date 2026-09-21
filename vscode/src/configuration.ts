import type { IIIClient } from 'iii-sdk';
import {
  type Config,
  type RuntimeConfig,
  RuntimeConfigSchema,
  runtimeJsonSchema,
  toRuntime,
} from './config.js';

const DEFAULT_CONFIG_ID = 'vscode';
const CONFIG_ID = process.env.III_CONFIG_NAME?.trim() || DEFAULT_CONFIG_ID;
const CONFIG_FN_ID = 'vscode::on-config-change';
const TIMEOUT_MS = 5_000;

export type ConfigHolder = { current: Config };

/**
 * Register VS Code settings and seed the candidate atomically via
 * `configuration::ensure`: the seed is forwarded unconditionally and the engine
 * installs it ONLY against an absent/null entry, so a stored operator/Compose
 * value is preserved without a client-side read-then-register race.
 */
export async function registerVscodeConfig(iii: IIIClient, seed: Config): Promise<void> {
  try {
    await iii.trigger({
      function_id: 'configuration::ensure',
      namespace: 'default',
      payload: {
        id: CONFIG_ID,
        name: 'VS Code',
        description:
          'VS Code worker: the code CLI path, the per-workspace data directory, the loopback bind host, the port range, and the start and stop timeouts.',
        schema: runtimeJsonSchema(),
        metadata: { ui_form: DEFAULT_CONFIG_ID },
        initial_value: toRuntime(seed),
      },
      timeoutMs: TIMEOUT_MS,
    });
  } catch (error) {
    if (isFunctionNotFound(error)) {
      throw new Error(ENSURE_UNAVAILABLE);
    }
    throw error;
  }
}

export const ENSURE_UNAVAILABLE =
  'configuration::ensure unavailable; upgrade engine with atomic configuration initialization support';

/** The engine's lowercase missing-FUNCTION code: an engine without configuration::ensure. */
function isFunctionNotFound(error: unknown): boolean {
  return (
    !!error && typeof error === 'object' && 'code' in error && error.code === 'function_not_found'
  );
}

/** Read and validate applied runtime settings, returning null when absent, invalid or unreachable. */
export async function fetchRuntime(iii: IIIClient): Promise<RuntimeConfig | null> {
  try {
    const res = await iii.trigger<unknown, { value?: unknown }>({
      function_id: 'configuration::get',
      namespace: 'default',
      payload: { id: CONFIG_ID, raw: false },
      timeoutMs: TIMEOUT_MS,
    });
    const value = res && typeof res === 'object' ? res.value : null;
    if (value == null) return null;
    return RuntimeConfigSchema.parse(value);
  } catch (err) {
    console.warn(`configuration::get failed for ${CONFIG_ID}: ${String(err)}`);
    return null;
  }
}

/** Apply an initial reload, then subscribe to updates for this VS Code instance's assigned entry. */
export async function bindConfigTrigger(
  iii: IIIClient,
  onChange: () => Promise<void>,
): Promise<void> {
  await onChange();
  iii.registerFunction(
    CONFIG_FN_ID,
    async () => {
      await onChange();
      return null;
    },
    {
      description: 'Internal: reload the vscode configuration when it changes.',
      metadata: { internal: true },
      request_format: { type: 'object', properties: {} },
      response_format: { type: 'null' },
    },
  );
  iii.registerTrigger({
    type: 'configuration',
    function_id: CONFIG_FN_ID,
    config: { configuration_id: CONFIG_ID, event_types: ['configuration:updated'] },
  });
}
