#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'hook-fanout',
  description:
    'Generic publish-collect primitive: publishes a topic via iii::durable::publish, collects subscriber replies on agent::hook_reply, applies a merge rule, returns the merged result.',
  register: (iii, ctx) => register(iii, ctx),
});
