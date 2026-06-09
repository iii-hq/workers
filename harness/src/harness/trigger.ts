/**
 * `harness::trigger` — browser kickoff for `run::start`.
 *
 * console/web sends `{ session_id?, message_id?, payload }`; this handler
 * stamps OTel baggage from the outer ids, then forwards `payload` to
 * `run::start` on the turn-orchestrator worker. Routing chat turns through
 * this function (instead of calling `run::start` directly) lets
 * `instrumentHandler` read `session_id` / `message_id` from the outer
 * request and stamp OTel baggage before the nested trigger runs, which
 * keeps "Group by session" / "Group by message" working in the traces UI
 * (`engine::traces::group_by`).
 */

import type { RemoteFunctionHandler } from 'iii-sdk';
import { z } from 'zod';
import type { ISdk } from '../runtime/iii.js';
import {
  RunStartPayloadSchema,
  type RunStartPayload,
  type RunStartResult,
} from '../turn-orchestrator/schemas.js';

const HarnessTriggerInputSchema = z.object({
  session_id: z.string().optional(),
  message_id: z.string().optional(),
  payload: RunStartPayloadSchema,
});

export type HarnessTriggerInput = z.infer<typeof HarnessTriggerInputSchema>;

export interface HarnessTriggerResponse {
  status_code: number;
  headers: Record<string, string>;
  body: RunStartResult;
}

/** Upper bound for a single `harness::trigger`. */
const BRIDGE_TIMEOUT_MS = 600_000;

export function register(iii: ISdk): void {
  const handler: RemoteFunctionHandler<HarnessTriggerInput, HarnessTriggerResponse> = async (
    input,
  ) => {
    const body = HarnessTriggerInputSchema.parse(input);
    const result = await iii.trigger<RunStartPayload, RunStartResult>({
      function_id: 'run::start',
      payload: body.payload,
      timeoutMs: BRIDGE_TIMEOUT_MS,
    });
    // A turn was already in flight: surface a 409 so the client can wait/retry
    // instead of treating the ignored kickoff as a started turn.
    return {
      status_code: result.started ? 200 : 409,
      headers: { 'content-type': 'application/json' },
      body: result,
    };
  };

  iii.registerFunction('harness::trigger', handler, {
    description:
      'Browser kickoff: forward payload to run::start. Used by console/web over the iii-browser-sdk.',
  });
}
