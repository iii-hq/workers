#!/usr/bin/env node
/**
 * Worker bootstrap: connect, register the configuration schema, fetch the live
 * config, register claude::* functions, wait for SIGINT/SIGTERM. Config is
 * managed by the `configuration` worker (config.yaml is the seed); the live
 * value hot-reloads. Mirrors the binary-worker lifecycle.
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

const { values } = parseArgs({
  options: {
    config: { type: 'string', default: './config.yaml' },
    url: { type: 'string' },
  },
  strict: false,
});

const seed = await loadConfig(String(values.config));
const url = values.url ? String(values.url) : seed.engine_url;

const iii = registerWorker(url, { workerName: 'claude-code' });

await registerClaudeConfig(iii, seed);

// Live snapshot: start from the seed, then refresh from the configuration
// worker. `claude_executable` is re-resolved on every refresh so a live change
// to it (or an empty value) re-runs the PATH lookup.
const holder: ConfigHolder = { current: seed };
const refresh = async () => {
  const runtime = (await fetchRuntime(iii)) ?? undefined;
  const merged: Config = runtime ? { engine_url: seed.engine_url, ...runtime } : { ...seed };
  merged.claude_executable = resolveClaudeExecutable(merged.claude_executable);
  holder.current = merged;
};

await bindConfigTrigger(iii, refresh);

// Emitters bind the boot stream names (a stream-name change needs a restart).
const emit = makeEmitter(iii, holder.current.events_stream);
const emitRaw = makeEmitter(iii, holder.current.raw_events_stream);
register(iii, () => holder.current, emit, emitRaw);

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
