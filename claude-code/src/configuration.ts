/**
 * Integration with the built-in `configuration` worker. `config.yaml` is the
 * seed installed as `initial_value` on first registration; the live value is
 * authoritative thereafter and hot-reloads on `configuration:updated`.
 *
 * Stream names (`events_stream` / `raw_events_stream`) are read once at boot to
 * build the emitters — a change to those needs a restart, like the path-jail
 * workers refuse a topology change. Every other field hot-reloads.
 */

import type { IIIClient } from 'iii-sdk';
import {
  type Config,
  type RuntimeConfig,
  RuntimeConfigSchema,
  runtimeJsonSchema,
  toRuntime,
} from './config.js';

const DEFAULT_CONFIG_ID = 'claude-code';
const CONFIG_ID = process.env.III_CONFIG_NAME?.trim() || DEFAULT_CONFIG_ID;
const CONFIG_FN_ID = 'claude::on-config-change';
const TIMEOUT_MS = 5_000;

/** Live snapshot shared with the handlers; `current` is whole-replaced on reload. */
export type ConfigHolder = { current: Config };

/** Refresh the Claude Code schema; send a seed only after a confirmed empty entry. */
export async function registerClaudeConfig(iii: IIIClient, seed: Config): Promise<void> {
  const initial = (await hasStoredValue(iii)) ? {} : { initial_value: toRuntime(seed) };
  await iii.trigger({
    function_id: 'configuration::register',
    namespace: 'default',
    payload: {
      id: CONFIG_ID,
      name: 'Claude Code',
      description:
        'Claude Code worker: per-turn defaults (model, permission mode, max turns, working directory, system-prompt append, allowed/disallowed tools), the agent::events / claude::events stream names, the approval-gate toggle, the claude CLI path, whether to inject the iii runtime context, and the terminal block — the binary, argv, workspace, and install/setup toggles for the console terminal page, which runs on the shell worker’s host, not this one.',
      schema: runtimeJsonSchema(),
      metadata: { ui_form: DEFAULT_CONFIG_ID },
      ...initial,
    },
    timeoutMs: TIMEOUT_MS,
  });
}

/** Never turn a service failure into permission to overwrite the stored value. */
async function hasStoredValue(iii: IIIClient): Promise<boolean> {
  try {
    const response = await iii.trigger<unknown, { value?: unknown }>({
      function_id: 'configuration::get',
      namespace: 'default',
      payload: { id: CONFIG_ID, raw: true },
      timeoutMs: TIMEOUT_MS,
    });
    return response?.value != null;
  } catch (error) {
    if (error && typeof error === 'object' && 'code' in error && error.code === 'NOT_FOUND') {
      return false;
    }
    throw error;
  }
}

/** Fetch the live runtime config; null when unset/unreachable. */
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

/**
 * Register the change handler + bind the `configuration` trigger. `onChange` is
 * called once now (reconcile) and on every `configuration:updated`.
 */
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
      description: 'Internal: reload claude-code configuration when it changes.',
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
