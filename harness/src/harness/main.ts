#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'harness',
  description:
    'Meta-worker: ui::subscribe/unsubscribe, harness::fs::read_inline, policy::check_permissions.',
  register: (iii, ctx) => register(iii, ctx),
});
