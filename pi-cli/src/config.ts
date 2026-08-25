/**
 * Settings live in the built-in `configuration` worker under the `pi-cli`
 * entry (Path B in docs/sops/configuration.md: no committed `config.yaml`,
 * defaults in code, the stored value authoritative and hot-reloading).
 */

import type { IIIClient } from 'iii-sdk';

const CONFIG_ID = 'pi-cli';
const CONFIG_FN_ID = 'pi-cli::on-config-change';
const TIMEOUT_MS = 5_000;

export type Config = {
  /** Path to the `pi` binary; empty resolves it on the terminal host. */
  executable: string;
  /** Extra argv for every session. */
  args: string[];
  /** Directory pi starts in; empty means `<primary shell root>/pi-cli`. */
  workspace_dir: string;
  /** Install pi (vendor installer) when the binary is missing. */
  auto_install: boolean;
  /** Keep the workspace equipped: iii skills, engine notes, activity hooks. */
  setup_workspace: boolean;
  /** Stream that carries the AgentEvent frames a session produces. */
  events_stream: string;
};

export const DEFAULTS: Config = {
  executable: '',
  // `-a` trusts the workspace for the run: without it pi asks about project
  // trust at every session and never loads this worker's own extension, which
  // is the thing that reports what pi did.
  args: ['-a'],
  workspace_dir: '',
  auto_install: true,
  setup_workspace: true,
  events_stream: 'agent::events',
};

export function jsonSchema(): Record<string, unknown> {
  return {
    type: 'object',
    additionalProperties: false,
    properties: {
      executable: {
        type: 'string',
        description:
          'Path to the pi binary on the terminal host. Empty: resolve `pi` on PATH there, installing it first when auto_install is on.',
      },
      args: {
        type: 'array',
        items: { type: 'string' },
        description:
          'Extra argv appended to every pi session. `-a` trusts the workspace for the run, which is what lets pi load the iii activity extension without asking.',
      },
      workspace_dir: {
        type: 'string',
        description:
          "Directory pi starts in. Empty: `pi-cli` under the shell worker's primary root. Must be reachable by the shell worker — it owns the terminal.",
      },
      auto_install: {
        type: 'boolean',
        description: 'Install pi from https://pi.dev/install.sh when it is missing.',
      },
      setup_workspace: {
        type: 'boolean',
        description:
          'Keep the workspace equipped on every boot: the iii skills, the engine notes, and the extension that reports activity.',
      },
      events_stream: {
        type: 'string',
        description:
          'Stream the AgentEvent frames land on, grouped by session id. Read once at boot.',
      },
    },
  };
}

/** Drop unknown keys rather than carry them: an older schema's value must not reach a spawn. */
export function normalize(value: unknown): Config {
  const stored = value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
  const config: Config = { ...DEFAULTS };
  for (const key of Object.keys(DEFAULTS) as (keyof Config)[]) {
    const incoming = stored[key];
    if (incoming === undefined) continue;
    // biome-ignore lint/suspicious/noExplicitAny: key-wise copy of a validated shape
    (config as any)[key] = incoming;
  }
  return config;
}

export type ConfigHolder = { current: Config };

export async function registerConfig(iii: IIIClient): Promise<void> {
  await iii.trigger({
    function_id: 'configuration::register',
    namespace: 'default',
    payload: {
      id: CONFIG_ID,
      name: 'Pi CLI',
      description:
        'pi terminal worker: the pi binary path and argv, the workspace directory, the install/setup toggles, and the stream its turns land on.',
      schema: jsonSchema(),
      initial_value: DEFAULTS,
    },
    timeoutMs: TIMEOUT_MS,
  });
}

export async function fetchConfig(iii: IIIClient): Promise<Config> {
  try {
    const res = await iii.trigger<unknown, { value?: unknown }>({
      function_id: 'configuration::get',
      namespace: 'default',
      payload: { id: CONFIG_ID, raw: false },
      timeoutMs: TIMEOUT_MS,
    });
    return normalize(res && typeof res === 'object' ? res.value : null);
  } catch (err) {
    console.warn(`configuration::get failed for ${CONFIG_ID}: ${String(err)}`);
    return { ...DEFAULTS };
  }
}

/** Calls `onChange` once now (reconcile) and on every `configuration:updated`. */
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
      description: 'Internal: reload pi-cli configuration when it changes.',
      request_format: { type: 'object', properties: {} },
      response_format: { type: 'null' },
      metadata: { internal: true, trace_hidden: true },
    },
  );
  iii.registerTrigger({
    type: 'configuration',
    function_id: CONFIG_FN_ID,
    config: { configuration_id: CONFIG_ID, event_types: ['configuration:updated'] },
  });
}
