#!/usr/bin/env node

import { parseArgs } from 'node:util';
import { registerWorker } from 'iii-sdk';
import { ProductionBridgeClientFactory } from './bridge.js';
import { type ConfigHolder, defaultConfig } from './config.js';
import { bindConfigTrigger, fetchRuntime, registerCursorConfig } from './configuration.js';
import { makeEmitter } from './events.js';
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
const emit = makeEmitter(iii, () => holder.current.events_stream);
const emitRaw = makeEmitter(iii, () => holder.current.raw_events_stream);
const worker = new CursorWorker(iii, () => holder.current, emit, emitRaw, factory);
worker.register();
await bindConfigTrigger(iii, holder);

console.log(`cursor worker connected to ${url}`);

let shuttingDown = false;
const shutdown = async () => {
  if (shuttingDown) return;
  shuttingDown = true;
  try {
    await worker.close();
    await iii.shutdown?.();
  } finally {
    process.exit(0);
  }
};

process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
process.on('exit', () => factory.forceCloseAll());
