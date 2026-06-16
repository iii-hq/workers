#!/usr/bin/env node
import { bootstrapWorker } from '../runtime/worker.js';
import { register } from './register.js';

await bootstrapWorker({
  name: 'context-compaction',
  description:
    'Thin user-initiated /compact wrapper (context-compaction::compact_session) over the context-manager worker; persists the compaction round trip as a session bookkeeping entry. The transcript is never modified.',
  register: (iii) => register(iii),
});
