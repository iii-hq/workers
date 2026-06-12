#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'provider-llamacpp',
  description:
    'llama.cpp llama-server (localhost) Chat Completions streaming provider on the iii bus (provider::llamacpp::stream).',
  register: (iii, ctx) => register(iii, ctx),
});
