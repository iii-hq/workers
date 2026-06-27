/**
 * Emit AgentEvent frames via the engine's stream builtin. Same per-process
 * epoch + per-session monotonic sequence scheme as the harness emitter, so
 * item_ids never collide across restarts.
 */

import { randomUUID } from 'node:crypto';
import type { IIIClient } from 'iii-sdk';
const PROCESS_EPOCH = randomUUID();
const seqBySession = new Map<string, number>();

export function makeEmitter(iii: IIIClient, streamName: string) {
  return async function emit(session_id: string, event: unknown): Promise<void> {
    const seq = seqBySession.get(session_id) ?? 0;
    seqBySession.set(session_id, seq + 1);
    const item_id = `${session_id}-${PROCESS_EPOCH}-${seq.toString().padStart(8, '0')}`;
    try {
      await iii.trigger({
        function_id: 'stream::set',
        payload: { stream_name: streamName, group_id: session_id, item_id, data: event },
      });
    } catch (err) {
      console.warn(`stream::set failed for ${session_id}: ${String(err)}`);
    }
  };
}

export type Emit = ReturnType<typeof makeEmitter>;
