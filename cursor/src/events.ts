import { randomUUID } from 'node:crypto';
import type { IIIClient } from 'iii-sdk';

const PROCESS_EPOCH = randomUUID();
const sequenceBySession = new Map<string, number>();

export function makeEmitter(iii: IIIClient, streamName: () => string) {
  return async function emit(
    sessionId: string,
    event: unknown,
    stableItemId?: string,
  ): Promise<boolean> {
    const sequence = sequenceBySession.get(sessionId) ?? 0;
    sequenceBySession.set(sessionId, sequence + 1);
    const itemId =
      stableItemId ?? `${sessionId}-${PROCESS_EPOCH}-${sequence.toString().padStart(8, '0')}`;
    try {
      await iii.trigger({
        function_id: 'stream::set',
        namespace: 'default',
        payload: {
          stream_name: streamName(),
          group_id: sessionId,
          item_id: itemId,
          data: event,
        },
      });
      return true;
    } catch (error) {
      console.warn(`cursor event delivery failed for ${sessionId}: ${safeError(error)}`);
      return false;
    }
  };
}

export type Emit = (sessionId: string, event: unknown, stableItemId?: string) => Promise<unknown>;

function safeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
