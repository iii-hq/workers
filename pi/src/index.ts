#!/usr/bin/env node
/**
 * Worker bootstrap: connect, register the configuration schema, fetch the live
 * config, register pi::* functions, wait for SIGINT/SIGTERM. Config is managed
 * by the `configuration` worker (config.yaml is the seed); the live value
 * hot-reloads. Mirrors the binary-worker lifecycle.
 *
 * Two halves, one worker: `pi::run` drives pi headless from here, and
 * `pi::terminal::*` drives the same agent in a `shell::pty` session a person
 * types into. Both report onto the same `agent::events` stream.
 */

import { parseArgs } from 'node:util';
import { registerWorker } from 'iii-sdk';
import { type Config, loadConfig } from './config.js';
import {
  bindConfigTrigger,
  type ConfigHolder,
  fetchRuntime,
  registerPiConfig,
} from './configuration.js';
import { makeEmitter } from './events.js';
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
const url = values.url
  ? String(values.url)
  : (process.env.III_URL ?? process.env.III_ENGINE_URL ?? seed.engine_url);
const bootConfig: Config = { ...seed, engine_url: url };

// The activity extension is loaded by pi's SDK for a headless turn as well
// (it discovers `.pi/extensions/` from the run's cwd, which IS the terminal
// workspace). This mark tells it so: the worker reports its own headless turns,
// and a second report from inside the same process would duplicate the run on
// `agent::events` under a different session id.
(globalThis as { __iiiPiWorker?: boolean }).__iiiPiWorker = true;

const iii = registerWorker(url, { workerName: 'pi' });

// Best-effort: a configuration-worker hiccup at boot must not stop the worker
// from registering pi::*; it falls back to the seed via fetchRuntime.
try {
  await registerPiConfig(iii, bootConfig);
} catch (err) {
  console.warn(`configuration::register failed; continuing with the seed: ${String(err)}`);
}

// Live snapshot: start from the seed, then refresh from the configuration worker.
const holder: ConfigHolder = { current: bootConfig };

// What a terminal session runs. The prepare step talks to the `shell` worker
// (install the CLI, equip the workspace, write the extension) and can take
// minutes on a cold host, so it is chained off the config reload rather than
// awaited: `pi::*` must register whether or not there is a terminal host to
// prepare. Reloads run one at a time, or a slower older prepare lands last and
// `pi::terminal::describe` answers with a workspace nobody configured any more.
let prepared: Prepared = {
  workspace: bootConfig.terminal.workspace_dir,
  executable: '',
  args: bootConfig.terminal.args,
  env: {},
  detail: 'the terminal host has not been prepared yet',
  bridge: '',
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
      if (prepared.detail) console.warn(`pi terminal: ${prepared.detail}`);
      else console.log(`pi terminal: ${prepared.executable} in ${prepared.workspace}`);
    })
    .catch((err) => {
      if (generation !== preparationGeneration) return;
      prepared = { ...prepared, executable: '', detail: String(err) };
      console.warn(`pi terminal: host is not ready: ${String(err)}`);
    });
};

const refresh = async () => {
  const runtime = (await fetchRuntime(iii)) ?? undefined;
  const merged: Config = runtime
    ? { engine_url: bootConfig.engine_url, ...runtime }
    : { ...bootConfig };
  holder.current = merged;
  // A new workspace or binary applies to the next session, without a restart.
  reconcileTerminal();
};

await bindConfigTrigger(iii, refresh);

// The stream names are read per event, so a live change applies to the next
// frame instead of waiting for a restart.
const emit = makeEmitter(iii, () => holder.current.events_stream);
const emitRaw = makeEmitter(iii, () => holder.current.raw_events_stream);
register(iii, () => holder.current, emit, emitRaw);

// The terminal half: the extension's event sink, what a session runs, who pays
// for it, and the console page that opens it.
registerActivity(iii, emit);
registerTerminal(iii, () => prepared);
registerAuth(iii, () => ({
  executable: prepared.executable,
  provider: holder.current.terminal.auth_provider,
  env: prepared.env,
}));
registerUi(iii);

console.log(`pi worker connected to ${url}`);

const shutdown = async () => {
  try {
    await iii.shutdown?.();
  } finally {
    process.exit(0);
  }
};
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
