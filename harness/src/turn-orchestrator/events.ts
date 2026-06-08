/**
 * Emit AgentEvent frames on `agent::events`, one per call with a per-session
 * monotonic sequence number. `turn_end` frames additionally enqueue a
 * compaction wake (`context-compaction::on_turn_end`) on the default queue, so
 * the out-of-band compactor runs once per turn instead of consuming the full
 * `agent::events` firehose. The enqueue is best-effort: a failure (e.g. the
 * compactor isn't deployed) is logged, never fatal.
 *
 * The sequence number is kept IN-PROCESS (not persisted): the old per-event
 * `state::update` increment cost one engine state write — and thus one
 * trigger-evaluation pass — per agent event (~66/turn). The seq only feeds the
 * stream frame `item_id`, which is opaque to consumers: the console never reads
 * it, the fanout strips it, and the engine delivers frames in insertion order
 * (item_id is just a `<group_id, item_id>` dedup key). So a process-local
 * counter is behavior-preserving. To keep item_ids unique across process
 * restarts without any persisted write, every item_id is prefixed with a
 * random per-process epoch — a fresh process can never collide with frames the
 * previous one wrote for the same session.
 */

import { uuidLike } from '../runtime/ids.js';
import { TriggerAction, type ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AgentEvent } from '../types/agent-event.js';

export const EVENTS_STREAM = 'agent::events';

/**
 * Out-of-band compactor woken once per turn. The turn-orchestrator enqueues a
 * typed `{ session_id, usage, provider, model }` payload here at `turn_end`
 * instead of mirroring the event onto a dedicated stream — a 1:1 channel is a
 * queue message, not pub/sub.
 */
const COMPACTION_ON_TURN_END = 'context-compaction::on_turn_end';
const COMPACTION_QUEUE = 'default';

/** Unique per process run; prefixes every item_id so a restart can't collide. */
const PROCESS_EPOCH = uuidLike();
/**
 * Per-session monotonic counter. One small integer per session seen by this
 * process; never reset across turns (matching the old persisted counter's
 * monotonic-per-session semantics), so item_ids stay unique within the process.
 */
const seqBySession = new Map<string, number>();

function nextSeq(session_id: string): number {
  const n = seqBySession.get(session_id) ?? 0;
  seqBySession.set(session_id, n + 1);
  return n;
}

/** Test seam: clear the in-process counters. Do not call in production. */
export function _resetSeqForTests(): void {
  seqBySession.clear();
}

function formatItemId(session_id: string, seq: number): string {
  return `${session_id}-${PROCESS_EPOCH}-${seq.toString().padStart(8, '0')}`;
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

/**
 * Enqueue an out-of-band compaction wake from a `turn_end` event. The assistant
 * message carries everything the compactor needs to decide overflow — token
 * usage plus the provider/model it ran on — so we pass them as a typed payload
 * rather than making the compactor re-derive them from a stream frame.
 */
async function enqueueCompaction(iii: ISdk, session_id: string, event: AgentEvent): Promise<void> {
  const message = (event as { message?: { usage?: unknown; provider?: unknown; model?: unknown } })
    .message;
  try {
    await iii.trigger({
      function_id: COMPACTION_ON_TURN_END,
      payload: {
        session_id,
        usage: message?.usage ?? null,
        provider: typeof message?.provider === 'string' ? message.provider : '',
        model: typeof message?.model === 'string' ? message.model : '',
      },
      action: TriggerAction.Enqueue({ queue: COMPACTION_QUEUE }),
    });
  } catch (err) {
    logger.warn('compaction on_turn_end enqueue failed', { session_id, err: String(err) });
  }
}

export async function emit(iii: ISdk, session_id: string, event: AgentEvent): Promise<void> {
  const seq = nextSeq(session_id);
  const item_id = formatItemId(session_id, seq);
  await setStream(iii, EVENTS_STREAM, session_id, item_id, event);
  if (isTurnEnd(event)) {
    await enqueueCompaction(iii, session_id, event);
  }
}
