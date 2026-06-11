#!/usr/bin/env node
/**
 * Worker bootstrap: connect to the engine, register claude::* functions,
 * wait for SIGINT/SIGTERM. Mirrors the binary-worker lifecycle from
 * iii-hq/workers (parse flags, registerWorker, register, shutdown).
 */

import { parseArgs } from 'node:util';
import { registerWorker } from 'iii-sdk';
import { loadConfig } from './config.js';
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

const cfg = await loadConfig(String(values.config));
cfg.claude_executable = resolveClaudeExecutable(cfg.claude_executable);
const url = values.url ? String(values.url) : cfg.engine_url;

const iii = registerWorker(url, { workerName: 'claude-code' });
const emit = makeEmitter(iii, cfg.events_stream);
register(iii, cfg, emit);

console.log(`claude-code worker connected to ${url}`);

const shutdown = async () => {
  await iii.shutdown?.();
  process.exit(0);
};
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
