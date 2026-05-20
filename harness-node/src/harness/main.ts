#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'harness',
  description:
    'Meta-worker: harness::status, ui::subscribe/unsubscribe, harness::fs::read_inline, policy::check_permissions, agent::events fanout to subscribed browsers.',
  register: (iii, ctx) => register(iii, ctx),
});
