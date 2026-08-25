#!/usr/bin/env node
/**
 * Worker bootstrap: connect, register the configuration schema, prepare the
 * terminal host (install the CLI, equip the workspace), register the
 * describe/activity/page surfaces, wait for SIGINT/SIGTERM.
 *
 * The terminal session itself belongs to the `shell` worker — this worker
 * decides WHAT runs in it and reports what the agent did.
 */

import { parseArgs } from 'node:util';
import { registerWorker } from 'iii-sdk';
import { registerActivity } from './activity.js';
import { registerAuth } from './auth.js';
import { bindConfigTrigger, type Config, fetchConfig, registerConfig } from './config.js';
import { makeEmitter } from './events.js';
import { registerTerminal } from './terminal.js';
import { registerUi } from './ui.js';
import { type Prepared, prepareWorkspace } from './workspace.js';

const { values } = parseArgs({ options: { url: { type: 'string' } }, strict: false });
const url = values.url
  ? String(values.url)
  : (process.env.III_URL ?? process.env.III_ENGINE_URL ?? 'ws://127.0.0.1:49134');

const iii = registerWorker(url, { workerName: 'claude-cli' });

// Best-effort: a configuration-worker hiccup at boot must not stop the worker
// from registering — the defaults are a working terminal.
try {
  await registerConfig(iii);
} catch (err) {
  console.warn(`configuration::register failed; continuing with defaults: ${String(err)}`);
}

let config: Config = await fetchConfig(iii);
let prepared: Prepared = {
  workspace: config.workspace_dir,
  executable: '',
  args: config.args,
  env: {},
  detail: 'the terminal host has not been prepared yet',
};

// The prepare step talks to the shell worker (install, skills, notes, hooks).
// It is re-run on every settings change, so a new workspace or binary applies
// to the next session without a restart.
const reconcile = async () => {
  config = await fetchConfig(iii);
  try {
    prepared = await prepareWorkspace(iii, config);
    if (prepared.detail) console.warn(`claude-cli: ${prepared.detail}`);
    else console.log(`claude-cli: ${prepared.executable} in ${prepared.workspace}`);
  } catch (err) {
    prepared = { ...prepared, executable: '', detail: String(err) };
    console.warn(`claude-cli: terminal host is not ready: ${String(err)}`);
  }
};

await bindConfigTrigger(iii, reconcile);

const emit = makeEmitter(iii, config.events_stream);
registerActivity(iii, emit);
registerTerminal(iii, () => prepared);
registerAuth(iii, () => prepared.executable);
registerUi(iii);

console.log(`claude-cli worker connected to ${url}`);

const shutdown = async () => {
  try {
    await iii.shutdown?.();
  } finally {
    process.exit(0);
  }
};
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
