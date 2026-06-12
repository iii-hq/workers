#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'provider-anthropic',
  description:
    'Anthropic Messages API streaming provider on the iii bus (provider::anthropic::stream).',
  register: (iii, ctx) => register(iii, ctx),
});
