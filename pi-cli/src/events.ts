/**
 * Emit AgentEvent frames via the engine's stream builtin. Same per-process
 * epoch + per-session monotonic sequence scheme as the harness and
 * claude-code emitters, so item_ids never collide across restarts.
 */

import { randomUUID } from 'node:crypto';
import type { IIIClient } from 'iii-sdk';
import type { AgentEvent } from './types.js';

const PROCESS_EPOCH = randomUUID();
const seqBySession = new Map<string, number>();

/**
 * `stream` may be a getter, and then it is read per event rather than captured:
 * the emitter outlives every configuration reload, and a captured name keeps
 * writing to a stream nobody reads any more.
 */
export function makeEmitter(iii: IIIClient, stream: string | (() => string)) {
  return async function emit(session_id: string, event: AgentEvent): Promise<void> {
    const stream_name = typeof stream === 'function' ? stream() : stream;
    const seq = seqBySession.get(session_id) ?? 0;
    seqBySession.set(session_id, seq + 1);
    const item_id = `${session_id}-${PROCESS_EPOCH}-${seq.toString().padStart(8, '0')}`;
    try {
      await iii.trigger({
        function_id: 'stream::set',
        payload: { stream_name, group_id: session_id, item_id, data: event },
      });
    } catch (err) {
      console.warn(`stream::set failed for ${session_id}: ${String(err)}`);
    }
  };
}

export type Emit = ReturnType<typeof makeEmitter>;
