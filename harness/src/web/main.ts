#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'web',
  description: 'Outbound HTTP client on the iii bus (web::fetch).',
  register: (iii, ctx) => register(iii, ctx),
});
