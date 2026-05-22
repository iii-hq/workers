/**
 * `turn::get_state` — one-shot reader for a session's current turn_state record.
 *
 * **Incoming**: flat `{ session_id }` from `console/web` real backend (page reload recovery)
 * **Outgoing**: `TurnStateRecord | null` — null when the session is unknown
 */

import { z } from 'zod';
import type { ISdk } from '../runtime/iii.js';
import * as persistence from './persistence.js';
import type { TurnStateRecord } from './state.js';

export const GetStatePayloadSchema = z.object({
  session_id: z.string().min(1),
});

export type GetStatePayload = z.infer<typeof GetStatePayloadSchema>;
export type GetStateResult = TurnStateRecord | null;

export async function execute(iii: ISdk, payload: GetStatePayload): Promise<GetStateResult> {
  return persistence.loadRecord(iii, payload.session_id);
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::get_state',
    async (payload: unknown) => execute(iii, GetStatePayloadSchema.parse(payload)),
    {
      description:
        'Read the current turn_state record for a session. Returns null if the session is unknown. UI clients use this on page reload to recover any in-progress modals (e.g. function_awaiting_approval) without reading iii state directly.',
    },
  );
}
