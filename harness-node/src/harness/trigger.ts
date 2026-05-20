/**
 * `harness::trigger` — browser kickoff for `run::start`.
 *
 * console/web sends `{ session_id?, message_id?, payload }`; this handler
 * stamps OTel baggage from the outer ids, then forwards `payload` to
 * `run::start` on the turn-orchestrator worker.
 */

import type { RemoteFunctionHandler } from 'iii-sdk';
import { z } from 'zod';
import type { ISdk } from '../runtime/iii.js';
import {
  RunStartPayloadSchema,
  type RunStartPayload,
  type RunStartResult,
} from '../turn-orchestrator/run-start.js';

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
    return {
      status_code: 200,
      headers: { 'content-type': 'application/json' },
      body: result,
    };
  };

  iii.registerFunction('harness::trigger', handler, {
    description:
      'Browser kickoff: forward payload to run::start. Used by console/web over the iii-browser-sdk.',
  });
}
