#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'provider-openai',
  description:
    'OpenAI Chat Completions streaming provider on the iii bus (provider::openai::stream + ::complete).',
  register: (iii, ctx) => register(iii, ctx),
});
