#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'approval-gate',
  description:
    'Owns approval state: registers approval::resolve and a state-trigger adapter on the approvals scope that wakes turn::step on every decision write.',
  register: (iii, ctx) => register(iii, ctx),
});
