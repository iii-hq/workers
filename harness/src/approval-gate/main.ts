#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './resolve.js';

await bootstrapWorker({
  name: 'approval-gate',
  description: 'Registers approval::resolve and routes decisions to per-call resume functions.',
  register: (iii) => register(iii),
});
