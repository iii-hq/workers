#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'provider-kimi',
  description:
    'Kimi (Moonshot) Chat Completions streaming provider on the iii bus (provider::kimi::stream + ::complete).',
  register: (iii, ctx) => register(iii, ctx),
});
