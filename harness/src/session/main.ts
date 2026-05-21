#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'session',
  description:
    'Session storage (parent-id tree under session-tree::*) and per-session inbox (session-inbox::*) backed by iii state.',
  register: (iii, ctx) => register(iii, ctx),
});
