#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'provider-config',
  description: 'Runtime provider settings store on the iii bus (provider_config::*).',
  register: (iii, ctx) => register(iii, ctx),
});
