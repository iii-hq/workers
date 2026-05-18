/**
 * `stream::set` wrapper used to emit AgentEvent / hook reply / arbitrary
 * stream frames. Mirrors `turn-orchestrator/src/events.rs` and the
 * approval-gate stream writers.
 */

import type { ISdk } from 'iii-sdk';
import { uuidLike } from './ids.js';
import { logger } from './otel.js';

export type StreamFrameInput = {
  stream_name: string;
  group_id: string;
  data: unknown;
  /** Optional explicit item id (otherwise a uuid-like one is minted). */
  item_id?: string;
};

export async function streamSet(iii: ISdk, frame: StreamFrameInput): Promise<void> {
  const item_id = frame.item_id ?? uuidLike();
  try {
    await iii.trigger<unknown, unknown>({
      function_id: 'stream::set',
      payload: {
        stream_name: frame.stream_name,
        group_id: frame.group_id,
        item_id,
        data: frame.data,
      },
    });
  } catch (err) {
    logger.warn('stream::set failed', {
      stream_name: frame.stream_name,
      group_id: frame.group_id,
      err: String(err),
    });
  }
}
