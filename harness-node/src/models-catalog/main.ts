#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'models-catalog',
  description: 'Model capabilities catalog on the iii bus (models::list/get/supports/register).',
  register: (iii) => register(iii),
});
