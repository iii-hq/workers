#!/usr/bin/env node

import { parseArgs } from 'node:util';
import { registerWorker } from 'iii-sdk';
import { createAgentFeeds } from './agent-feed.js';
import { ProductionBridgeClientFactory } from './bridge.js';
import { ProductionCursorCliFactory } from './cli.js';
import { type ConfigHolder, defaultConfig } from './config.js';
import { bindConfigTrigger, fetchRuntime, registerCursorConfig } from './configuration.js';
import { CursorProvider } from './provider.js';
import { CursorWorker } from './run.js';

const { values } = parseArgs({
  options: { url: { type: 'string' } },
  strict: false,
});

const url =
  (values.url ? String(values.url) : undefined) ??
  process.env.III_URL ??
  process.env.III_ENGINE_URL ??
  'ws://127.0.0.1:49134';

const iii = registerWorker(url, { workerName: 'cursor' });
await registerCursorConfig(iii, defaultConfig());
const holder: ConfigHolder = { current: await fetchRuntime(iii) };
const factory = new ProductionBridgeClientFactory();
const cliFactory = new ProductionCursorCliFactory();
// Owned feeds: cursor::agent-event and cursor::raw-event (bind with { session_id }).
const { emit, emitRaw } = createAgentFeeds(iii);
const worker = new CursorWorker(iii, () => holder.current, emit, emitRaw, factory, cliFactory);
const provider = new CursorProvider(iii, () => holder.current, cliFactory);
worker.register();
provider.register();
await bindConfigTrigger(iii, holder);

console.log(`cursor worker connected to ${url}`);

let shuttingDown = false;
const shutdown = async () => {
  if (shuttingDown) return;
  shuttingDown = true;
  const watchdog = setTimeout(() => process.exit(1), 15_000);
  watchdog.unref();
  try {
    await provider.close();
    await worker.close();
    await iii.shutdown?.();
  } finally {
    clearTimeout(watchdog);
    process.exit(0);
  }
};

process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
process.on('exit', () => worker.forceClose());
