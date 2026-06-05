#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'models-catalog',
  description: 'Model capabilities catalog on the iii bus (models::list/get/supports/reconcile).',
  register: async (iii) => register(iii),
});
