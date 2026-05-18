#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'approval-gate',
  description:
    'Hook subscriber on agent::before_function_call that consults policy::check_permissions and pauses calls for user approval.',
  register: (iii, ctx) => register(iii, ctx),
});
