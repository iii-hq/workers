#!/usr/bin/env node
/**
 * Worker bootstrap: connect, register the configuration schema, fetch the live
 * config, register opencode::* functions, wait for SIGINT/SIGTERM. Config is
 * managed by the `configuration` worker (config.yaml is the seed); the live
 * value hot-reloads.
 */

import { parseArgs } from 'node:util';
import { registerWorker } from 'iii-sdk';
import { type Config, loadConfig } from './config.js';
import {
  bindConfigTrigger,
  type ConfigHolder,
  fetchRuntime,
  registerOpencodeConfig,
} from './configuration.js';
import { makeEmitter } from './events.js';
import { resolveOpencodeExecutable } from './executable.js';
import { register } from './run.js';

const { values } = parseArgs({
  options: {
    config: { type: 'string', default: './config.yaml' },
    url: { type: 'string' },
  },
  strict: false,
});

const seed = await loadConfig(String(values.config));
const url = values.url ? String(values.url) : seed.engine_url;

const iii = registerWorker(url, { workerName: 'opencode' });

try {
  await registerOpencodeConfig(iii, seed);
} catch (err) {
  console.warn(`configuration::register failed; continuing with the seed: ${String(err)}`);
}

const holder: ConfigHolder = { current: seed };
const refresh = async () => {
  const runtime = (await fetchRuntime(iii)) ?? undefined;
  const merged: Config = runtime ? { engine_url: seed.engine_url, ...runtime } : { ...seed };
  merged.opencode_executable = resolveOpencodeExecutable(merged.opencode_executable);
  holder.current = merged;
};

await bindConfigTrigger(iii, refresh);

const emit = makeEmitter(iii, holder.current.events_stream);
const emitRaw = makeEmitter(iii, holder.current.raw_events_stream);
register(iii, () => holder.current, emit, emitRaw);

console.log(`opencode worker connected to ${url}`);

const shutdown = async () => {
  try {
    await iii.shutdown?.();
  } finally {
    process.exit(0);
  }
};
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
