/**
 * Integration with the built-in `configuration` worker. `config.yaml` is the
 * seed installed as `initial_value` on first registration; the live value is
 * authoritative thereafter and hot-reloads on `configuration:updated`.
 *
 * Every field hot-reloads. The event feeds are fixed functions
 * (`claude::on-agent-event`, `claude::on-raw-event`), not configuration; the
 * former `events_stream` / `raw_events_stream` keys are accepted and ignored.
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
const ON_UPDATED_FN = 'configuration::on-updated';
const TIMEOUT_MS = 5_000;

/** Live snapshot shared with the handlers; `current` is whole-replaced on reload. */
export type ConfigHolder = { current: Config };

/**
 * Refresh the Claude Code schema and seed the candidate atomically via
 * `configuration::ensure`: the seed is forwarded unconditionally and the engine
 * installs it ONLY against an absent/null entry, so a stored operator/Compose
 * value is preserved without a client-side read-then-register race on modern engines.
 * Engines lacking ensure use the warned, non-atomic legacy compatibility path.
 */
export async function registerClaudeConfig(iii: IIIClient, seed: Config): Promise<void> {
  const payload: Record<string, unknown> = {
    id: CONFIG_ID,
    name: 'Claude Code',
    description:
      'Claude Code worker: per-turn defaults (model, permission mode, max turns, working directory, system-prompt append, allowed/disallowed tools), the approval-gate toggle, the claude CLI path, whether to inject the iii runtime context, and the terminal block — the binary, argv, workspace, and install/setup toggles for the console terminal page, which runs on the ide worker’s host, not this one.',
    schema: runtimeJsonSchema(),
    metadata: { ui_form: DEFAULT_CONFIG_ID },
    initial_value: toRuntime(seed),
  };
  await ensureConfiguration(iii, payload);
}

let warnedLegacy = false;

/** Missing ensure alone authorizes this non-atomic compatibility path. */
async function ensureConfiguration(
  iii: IIIClient,
  payload: Record<string, unknown>,
): Promise<void> {
  const call = (function_id: string, payload: Record<string, unknown>) =>
    iii.call({ function_id, namespace: 'default', payload, timeoutMs: TIMEOUT_MS });
  try {
    await call('configuration::ensure', payload);
    return;
  } catch (error) {
    if (!hasCode(error, 'function_not_found')) throw error;
  }
  if (!warnedLegacy) {
    warnedLegacy = true;
    console.warn(
      `${CONFIG_ID}: engine lacks configuration::ensure; using non-atomic legacy initialization; upgrade to >=0.24.1 for concurrent-write safety`,
    );
  }
  let existing: unknown;
  try {
    const response = await call('configuration::get', { id: payload.id, raw: true });
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
    if (!hasCode(error, 'NOT_FOUND')) throw error;
    existing = null;
  }
  const registration = { ...payload };
  if (existing !== null) delete registration.initial_value;
  await call('configuration::register', registration);
}

/** Inspect the SDK code, never message text. */
function hasCode(error: unknown, code: string): boolean {
  const functionId = code === 'function_not_found' ? 'configuration::ensure' : 'configuration::get';
  return (
    !!error &&
    typeof error === 'object' &&
    'code' in error &&
    error.code === code &&
    (!('function_id' in error) ||
      error.function_id === undefined ||
      error.function_id === functionId)
  );
}

/** A lease on a confirmed live value, never on a seed or a fallback. Starting a
 * refresh revokes outstanding leases before waiting on the configuration worker. */
export class ReplyPermission {
  private generation = 0;
  epoch: number | null = null;

  invalidate() {
    this.generation++;
    this.epoch = null;
  }

  async refresh(iii: IIIClient, updatesActive: () => boolean = () => true) {
    this.invalidate();
    const generation = this.generation;
    const runtime = await fetchRuntime(iii);
    if (generation !== this.generation) return undefined;
    if (runtime?.external_sessions === true && updatesActive()) this.epoch = generation;
    return { runtime };
  }
}

/** Fetch the live runtime config; null when unset/unreachable. */
export async function fetchRuntime(iii: IIIClient): Promise<RuntimeConfig | null> {
  try {
    const res = await iii.call<unknown, { value?: unknown }>({
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
 * Register the change handler and observe `configuration::on-updated` for this
 * entry. `onChange` is called once now (reconcile) and on every
 * `configuration:updated`. When tracking reply permission, also reconcile it
 * on registration and deletion without reconfiguring unrelated settings.
 */
export async function observeConfigChanges(
  iii: IIIClient,
  onChange: (reloadSettings?: boolean) => Promise<void>,
  onWatchState?: (active: boolean) => void,
): Promise<void> {
  iii.registerFunction(
    CONFIG_FN_ID,
    async ({ output }: { output?: { event_type?: string } }) => {
      const lifecycle =
        output?.event_type === 'configuration:deleted' ||
        output?.event_type === 'configuration:registered';
      await onChange(!lifecycle);
      return null;
    },
    {
      description: 'Internal: reload claude-code configuration when it changes.',
      metadata: { internal: true },
      request_format: { type: 'object', properties: {} },
      response_format: { type: 'null' },
    },
  );
  const watch = iii.observe(
    {
      function_id: ON_UPDATED_FN,
      filters: {
        'input.id': { $eq: CONFIG_ID },
        ...(onWatchState ? {} : { 'input.event_type': { $eq: 'configuration:updated' } }),
      },
      capture: ['output'],
    },
    CONFIG_FN_ID,
  );
  // A disconnect can hide a disable/re-enable transition. Do not trust the
  // previous value across that gap; reconcile again once updates can arrive.
  if (onWatchState) {
    watch.onStateChange((state) => {
      onWatchState(state === 'active');
      if (state === 'active') void onChange();
    });
    onWatchState(watch.state === 'active');
  }
  await onChange();
}
