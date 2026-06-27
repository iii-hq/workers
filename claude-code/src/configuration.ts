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

const CONFIG_ID = 'claude-code';
const CONFIG_FN_ID = 'claude::on-config-change';
const TIMEOUT_MS = 5_000;

/** Live snapshot shared with the handlers; `current` is whole-replaced on reload. */
export type ConfigHolder = { current: Config };

export async function registerClaudeConfig(iii: IIIClient, seed: Config): Promise<void> {
  await iii.trigger({
    function_id: 'configuration::register',
    payload: {
      id: CONFIG_ID,
      name: 'Claude Code',
      description:
        'Claude Code worker: per-turn defaults (model, permission mode, max turns, working directory, system-prompt append, allowed/disallowed tools), the agent::events / claude::events stream names, the approval-gate toggle, the claude CLI path, and whether to inject the iii runtime context.',
      schema: runtimeJsonSchema(),
      initial_value: toRuntime(seed),
    },
    timeoutMs: TIMEOUT_MS,
  });
}

/** Fetch the live runtime config; null when unset/unreachable. */
export async function fetchRuntime(iii: IIIClient): Promise<RuntimeConfig | null> {
  try {
    const res = await iii.trigger<unknown, { value?: unknown }>({
      function_id: 'configuration::get',
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
