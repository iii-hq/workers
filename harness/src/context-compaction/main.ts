#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'context-compaction',
  description:
    'Out-of-band session-history compactor. Woken by a turn-orchestrator queue message at turn_end; writes a session-tree Compaction entry when the running token count crosses the configured threshold.',
  register: (iii) => register(iii),
});
