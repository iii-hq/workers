#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'llm-budget',
  description: 'LLM spend caps with alerts, forecast, period rollover (budget::*).',
  register: (iii) => register(iii),
});
