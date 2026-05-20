#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'auth-credentials',
  description: 'Credential store for provider API keys and OAuth tokens (auth::*).',
  register: (iii, ctx) => register(iii, ctx),
});
