#!/usr/bin/env node
/**
 * Worker bootstrap: connect, register the configuration schema, fetch the live
 * config, register claude::* functions, wait for SIGINT/SIGTERM. Config is
 * managed by the `configuration` worker (config.yaml is the seed); the live
 * value hot-reloads. Mirrors the binary-worker lifecycle.
 *
 * Two halves, one worker and one login: `claude::run` drives Claude Code
 * headless from here, and `claude::terminal::*` drives the same CLI in a
 * `shell::pty` session a person types into. Both report onto the same
 * `agent::events` stream.
 */

import { parseArgs } from 'node:util';
import { registerWorker } from 'iii-sdk';
import { type Config, loadConfig } from './config.js';
import {
  bindConfigTrigger,
  type ConfigHolder,
  fetchRuntime,
  registerClaudeConfig,
} from './configuration.js';
import { makeEmitter } from './events.js';
import { resolveClaudeExecutable } from './executable.js';
import { register } from './run.js';
import { registerActivity } from './terminal/activity.js';
import { registerAuth } from './terminal/auth.js';
import { registerTerminal } from './terminal/terminal.js';
import { registerUi } from './terminal/ui.js';
import { type Prepared, prepareWorkspace } from './terminal/workspace.js';

const { values } = parseArgs({
  options: {
    config: { type: 'string', default: './config.yaml' },
    url: { type: 'string' },
  },
  strict: false,
});

const seed = await loadConfig(String(values.config));
const url =
  (values.url ? String(values.url) : undefined) ??
  process.env.III_URL ??
  process.env.III_ENGINE_URL ??
  seed.engine_url;
const bootConfig: Config = {
  ...seed,
  engine_url: url,
  claude_executable: resolveClaudeExecutable(seed.claude_executable),
};

const iii = registerWorker(url, { workerName: 'claude-code' });

// Best-effort: a configuration-worker hiccup at boot must not stop the worker
// from registering claude::*; it falls back to the seed via fetchRuntime.
try {
  await registerClaudeConfig(iii, bootConfig);
} catch (err) {
  console.warn(`configuration::register failed; continuing with the seed: ${String(err)}`);
}

// Live snapshot: start from the seed, then refresh from the configuration
// worker. `claude_executable` is re-resolved on every refresh so a live change
// to it (or an empty value) re-runs the PATH lookup.
const holder: ConfigHolder = { current: bootConfig };

// What a terminal session runs. The prepare step talks to the `shell` worker
// (install the CLI, equip the workspace, write the hooks) and can take minutes
// on a cold host, so it is chained off the config reload rather than awaited:
// `claude::*` must register whether or not there is a terminal host to prepare.
let prepared: Prepared = {
  workspace: bootConfig.terminal.workspace_dir,
  executable: '',
  args: bootConfig.terminal.args,
  env: {},
  detail: 'the terminal host has not been prepared yet',
  bridge: '',
  plugin: '',
};
let preparing: Promise<void> = Promise.resolve();
// A queued reconcile must not let a slower, superseded preparation overwrite
// a newer one's result: each run is stamped with the generation current when
// it started, and only commits `prepared` if that generation is still current.
let preparationGeneration = 0;
const reconcileTerminal = () => {
  const generation = ++preparationGeneration;
  const config = holder.current.terminal;
  preparing = preparing
    .then(async () => {
      const candidate = await prepareWorkspace(iii, config);
      if (generation !== preparationGeneration) return;
      prepared = candidate;
      if (prepared.detail) console.warn(`claude-code terminal: ${prepared.detail}`);
      else console.log(`claude-code terminal: ${prepared.executable} in ${prepared.workspace}`);
    })
    .catch((err) => {
      if (generation !== preparationGeneration) return;
      prepared = { ...prepared, executable: '', detail: String(err) };
      console.warn(`claude-code terminal: host is not ready: ${String(err)}`);
    });
};

const refresh = async () => {
  const runtime = (await fetchRuntime(iii)) ?? undefined;
  const merged: Config = runtime
    ? { engine_url: bootConfig.engine_url, ...runtime }
    : { ...bootConfig };
  merged.claude_executable = resolveClaudeExecutable(merged.claude_executable);
  holder.current = merged;
  // A new workspace or binary applies to the next session, without a restart.
  reconcileTerminal();
};

await bindConfigTrigger(iii, refresh);

// Emitters bind the boot stream names (a stream-name change needs a restart).
const emit = makeEmitter(iii, holder.current.events_stream);
const emitRaw = makeEmitter(iii, holder.current.raw_events_stream);
register(iii, () => holder.current, emit, emitRaw);

// The terminal half: the hook sink, what a session runs, who pays for it, and
// the console page that opens it.
registerActivity(iii, emit);
registerTerminal(iii, () => prepared);
registerAuth(iii, () => prepared);
registerUi(iii);

console.log(`claude-code worker connected to ${url}`);

const shutdown = async () => {
  try {
    await iii.shutdown?.();
  } finally {
    process.exit(0);
  }
};
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
