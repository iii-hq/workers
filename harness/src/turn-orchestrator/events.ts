/**
 * Emit AgentEvent frames on `agent::events`, one per call with a per-session
 * monotonic sequence number. `turn_end` frames are additionally mirrored onto
 * the dedicated `agent::turn_end` stream (see TURN_END_STREAM).
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AgentEvent } from '../types/agent-event.js';
import { AGENT_SCOPE, eventCounterKey } from './state.js';

export const EVENTS_STREAM = 'agent::events';
/**
 * Dedicated stream carrying only `turn_end` frames. Compaction subscribes here
 * instead of the full `agent::events` firehose so it wakes once per turn rather
 * than on every event (token updates, function lifecycle, …).
 */
export const TURN_END_STREAM = 'agent::turn_end';

function formatItemId(session_id: string, seq: number): string {
  return `${session_id}-${seq.toString().padStart(8, '0')}`;
}

function isTurnEnd(event: AgentEvent): boolean {
  return (event as { type?: string }).type === 'turn_end';
}

async function setStream(
  iii: ISdk,
  stream_name: string,
  session_id: string,
  item_id: string,
  event: AgentEvent,
): Promise<void> {
  try {
    await iii.trigger<unknown, unknown>({
      function_id: 'stream::set',
      payload: { stream_name, group_id: session_id, item_id, data: event },
    });
  } catch (err) {
    logger.warn('stream::set failed', {
      stream_name,
      session_id,
      item_id,
      err: String(err),
    });
  }
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
  await setStream(iii, EVENTS_STREAM, session_id, item_id, event);
  if (isTurnEnd(event)) {
    await setStream(iii, TURN_END_STREAM, session_id, item_id, event);
  }
}
