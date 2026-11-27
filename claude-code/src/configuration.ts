/**
 * Integration with the built-in `configuration` worker. `config.yaml` is the
 * seed installed as `initial_value` on first registration; the live value is
 * authoritative thereafter and hot-reloads on `configuration:updated`.
 *
 * Every field hot-reloads. The event feeds are fixed functions
 * (`claude::on-agent-event`, `claude::on-raw-event`), not configuration; the
 * former `events_stream` / `raw_events_stream` keys are accepted and ignored.
 */

import type { IIIClient, ObserverHandle } from 'iii-sdk';
import {
  type Config,
  type RuntimeConfig,
  RuntimeConfigSchema,
  runtimeJsonSchema,
  toRuntime,
} from './config.js';

const DEFAULT_CONFIG_ID = 'claude-code';
export const CONFIG_ID = process.env.III_CONFIG_NAME?.trim() || DEFAULT_CONFIG_ID;
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

type ConfirmOptions = { waitForWatchRecovery?: boolean };
export type ReplyAccess = {
  readonly epoch: number | null;
  confirm(options?: ConfirmOptions): Promise<number | null>;
};

/** Bind a series of reads to one grant. Even a rejected inventory lookup can
 * disclose whether a session exists; check the grant before and after each read. */
export function replyLease(access: ReplyAccess, epoch: number | null) {
  const check = () => {
    if (epoch === null || epoch !== access.epoch) throw new Error('reply access disabled');
    return epoch;
  };
  return {
    epoch: check(),
    check,
    async read<T>(read: () => Promise<T>) {
      check();
      try {
        return await read();
      } finally {
        check();
      }
    },
  };
}

/** A lease on a confirmed live value, never on a seed or a fallback. Starting a
 * refresh revokes outstanding leases before waiting on the configuration worker. */
export class ReplyPermission {
  private generation = 0;
  private currentEpoch: number | null = null;
  private refreshConnection: number | null = null;
  private reloading = false;
  private confirmation?: Promise<number | null>;
  private watchRecovery?: Promise<void>;
  private recoverWatches?: (epoch: number) => Promise<void>;
  private watch?: { iii: IIIClient; handle: ObserverHandle };

  trackWatch(iii: IIIClient, handle: ObserverHandle) {
    this.invalidate();
    this.watch = { iii, handle };
    handle.onStateChange((state) => {
      if (state === 'active') void this.refresh(iii);
      else this.invalidate();
    });
  }

  private watchConnection() {
    if (!this.watch) return null;
    const { iii, handle } = this.watch;
    // SDK 0.27 preserves the handle state across socket loss and sends no
    // state change for active-to-active replay. Check its connection and
    // replay acknowledgement too; a missing SDK signal fails closed.
    if (
      handle.state !== 'active' ||
      !('answered' in handle && handle.answered === true) ||
      !('observeOnline' in iii && typeof iii.observeOnline === 'function' && iii.observeOnline()) ||
      !('connectionEpoch' in iii && typeof iii.connectionEpoch === 'number')
    ) {
      return null;
    }
    return iii.connectionEpoch;
  }

  get epoch() {
    const connection = this.watchConnection();
    if (connection !== this.refreshConnection) this.invalidate();
    return this.currentEpoch;
  }

  invalidate() {
    this.generation++;
    this.currentEpoch = null;
    this.refreshConnection = null;
    this.reloading = false;
    this.confirmation = undefined;
    this.watchRecovery = undefined;
  }

  onConfirmedRecovery(recover: (epoch: number) => Promise<void>) {
    this.recoverWatches = recover;
  }

  // An explicit probe, admission or poll can recover on the same connection.
  // Concurrent readers share a probe; a config event always supersedes it.
  async confirm({ waitForWatchRecovery = true }: ConfirmOptions = {}): Promise<number | null> {
    const connection = this.watchConnection();
    if (connection !== this.refreshConnection) this.invalidate();
    if (!this.watch || connection === null || this.reloading) return null;
    if (!this.confirmation) {
      const generation = this.generation;
      this.refreshConnection = connection;
      this.confirmation = fetchRuntime(this.watch.iii).then((runtime) => {
        if (generation !== this.generation) return null;
        this.confirmation = undefined;
        if (connection !== this.watchConnection() || runtime?.external_sessions !== true) {
          this.invalidate();
          return null;
        }
        this.currentEpoch = generation;
        // Share rebaselining separately so inventory callers need only the
        // live config read. A slow recovery must not cache its positive probe.
        if (!this.watchRecovery && this.recoverWatches) {
          this.watchRecovery = this.recoverWatches(generation)
            .catch(() => {
              // An authorized poll can still rebaseline a stale watch.
            })
            .finally(() => {
              if (generation === this.generation) this.watchRecovery = undefined;
            });
        }
        return generation;
      });
    }
    const epoch = await this.confirmation;
    if (epoch === null || epoch !== this.epoch) return null;
    if (waitForWatchRecovery) await this.watchRecovery;
    return epoch === this.epoch ? epoch : null;
  }

  async refresh(iii: IIIClient) {
    this.invalidate();
    const generation = this.generation;
    const connection = this.watchConnection();
    this.refreshConnection = connection;
    this.reloading = true;
    const runtime = await fetchRuntime(iii);
    if (generation === this.generation) this.reloading = false;
    if (
      generation === this.generation &&
      runtime?.external_sessions === true &&
      connection !== null &&
      connection === this.watchConnection()
    ) {
      this.currentEpoch = generation;
    }
    return runtime;
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
  trackWatch?: (handle: ObserverHandle) => void,
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
        ...(trackWatch ? {} : { 'input.event_type': { $eq: 'configuration:updated' } }),
      },
      capture: ['output'],
    },
    CONFIG_FN_ID,
  );
  trackWatch?.(watch);
  await onChange();
}

/** Settings and reply permission share a read, but not a supersession policy.
 * Lifecycle events and watch gaps revoke reply leases without changing which
 * configuration update is eligible to apply ordinary settings. */
export function observeReplyConfig(
  iii: IIIClient,
  permission: ReplyPermission,
  applySettings: (runtime: RuntimeConfig | null) => void,
) {
  let settingsGeneration = 0;
  return observeConfigChanges(
    iii,
    async (reloadSettings = true) => {
      const generation = reloadSettings ? ++settingsGeneration : undefined;
      const runtime = await permission.refresh(iii);
      if (generation !== undefined && generation === settingsGeneration) applySettings(runtime);
    },
    (watch) => permission.trackWatch(iii, watch),
  );
}
