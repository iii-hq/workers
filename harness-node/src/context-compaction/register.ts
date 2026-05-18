/**
 * Register the agent::events subscriber. Mirrors
 * `context-compaction/src/lib.rs::register_with_iii`.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { extractEventPayload, turnEndUsage, usageTotal } from './handler.js';
import { acquireLease, releaseLease } from './lease.js';
import { summarizeAndAppend } from './summarize.js';
import { shouldCompact } from './threshold.js';

const SUBSCRIBER_FN = 'context-compaction::on_agent_event';
const AGENT_EVENTS_STREAM = 'agent::events';

async function handleEvent(iii: ISdk, frame: unknown): Promise<void> {
  const payload = extractEventPayload(frame);
  if (!payload) return;
  const usage = turnEndUsage(payload.event);
  if (!usage) return;
  if (!shouldCompact(usageTotal(usage))) return;
  const nonce = await acquireLease(iii, payload.session_id);
  if (!nonce) {
    logger.debug('compaction lease held; skipping', { session_id: payload.session_id });
    return;
  }
  try {
    await summarizeAndAppend(iii, payload.session_id);
  } catch (err) {
    logger.warn('compaction failed', { session_id: payload.session_id, err: String(err) });
  } finally {
    await releaseLease(iii, payload.session_id, nonce);
  }
}

export async function register(iii: ISdk): Promise<void> {
  iii.registerFunction(
    SUBSCRIBER_FN,
    async (frame: unknown) => {
      await handleEvent(iii, frame);
      return null;
    },
    {
      description:
        'Internal: subscribes to agent::events; triggers session compaction on TurnEnd when running tokens exceed the configured threshold.',
    },
  );
  iii.registerTrigger({
    type: 'stream',
    function_id: SUBSCRIBER_FN,
    config: { stream_name: AGENT_EVENTS_STREAM },
  });
}
