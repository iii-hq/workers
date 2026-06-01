#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './resolve.js';
import { initDefaultMode } from './settings/default-mode.js';
import { registerSettingsHandlers } from './settings/register.js';

await bootstrapWorker({
  name: 'approval-gate',
  description:
    'Registers approval::resolve, per-session settings handlers, and routes decisions to the orchestrator.',
  register: async (iii) => {
    await register(iii);
    registerSettingsHandlers(iii);
    await initDefaultMode(iii);
  },
});
