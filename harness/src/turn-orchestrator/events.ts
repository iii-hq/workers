/**
 * Emit AgentEvent frames on `agent::events`, one per call with a per-session
 * monotonic sequence number.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AgentEvent } from '../types/agent-event.js';
import { AGENT_SCOPE, eventCounterKey } from './state.js';

export const EVENTS_STREAM = 'agent::events';

function formatItemId(session_id: string, seq: number): string {
  return `${session_id}-${seq.toString().padStart(8, '0')}`;
}

async function nextSeq(iii: ISdk, session_id: string): Promise<number> {
  try {
    const resp = await iii.trigger<unknown, { old_value?: number }>({
      function_id: 'state::update',
      payload: {
        scope: AGENT_SCOPE,
        key: eventCounterKey(session_id),
        ops: [{ type: 'increment', path: '', by: 1 }],
      },
    });
    if (typeof resp?.old_value === 'number') return resp.old_value;
  } catch (err) {
    logger.warn('event_counter increment failed', {
      session_id,
      err: String(err),
    });
  }
  return 0;
}

export async function emit(iii: ISdk, session_id: string, event: AgentEvent): Promise<void> {
  const seq = await nextSeq(iii, session_id);
  const item_id = formatItemId(session_id, seq);
  try {
    await iii.trigger<unknown, unknown>({
      function_id: 'stream::set',
      payload: {
        stream_name: EVENTS_STREAM,
        group_id: session_id,
        item_id,
        data: event,
      },
    });
  } catch (err) {
    logger.warn('stream::set agent::events failed', {
      session_id,
      item_id,
      err: String(err),
    });
  }
}
