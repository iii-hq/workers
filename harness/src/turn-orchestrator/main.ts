#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'turn-orchestrator',
  description:
    'Durable run::start state machine driving each agent turn through provisioning, assistant, and function-execute.',
  register: (iii, ctx) => register(iii, ctx),
});
