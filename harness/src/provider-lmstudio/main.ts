#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'provider-lmstudio',
  description:
    'LM Studio (localhost) Chat Completions streaming provider on the iii bus (provider::lmstudio::stream + ::complete).',
  register: (iii, ctx) => register(iii, ctx),
});
