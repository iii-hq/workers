/**
 * `turn::step` — one FSM transition for a session.
 *
 * **Incoming** (both paths deliver the same flat shape):
 * - Direct `iii.trigger`: `{ session_id }` from `on-record-written`, `approval-resume`
 * - `durable:subscriber` on `turn::step_requested`: `{ session_id }` only — producers
 *   call `iii::durable::publish` with `{ topic, data: { session_id } }` but the engine
 *   enqueues `data`, not the publish envelope
 *
 * **Outgoing**: `StepResult` — never throws for unknown/terminal; throws on transition failure
 */

import { z } from 'zod';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { TurnOrchestratorConfig } from './config.js';
import * as persistence from './persistence.js';
import { isTerminal, type TurnState } from './state.js';
import { step } from './transitions.js';

export const StepPayloadSchema = z.object({
  session_id: z.string().min(1),
});

export type StepPayload = z.infer<typeof StepPayloadSchema>;

export type StepResult =
  | { ok: true; terminal: true }
  | { ok: true; from_state: TurnState; to_state: TurnState }
  | { ok: false; reason: 'unknown_session' };

export async function execute(
  iii: ISdk,
  cfg: TurnOrchestratorConfig,
  payload: StepPayload,
): Promise<StepResult> {
  const { session_id } = payload;
  const rec = await persistence.loadRecord(iii, session_id);
  if (!rec) {
    logger.warn('turn::step for unknown session', { session_id });
    return { ok: false, reason: 'unknown_session' };
  }
  if (isTerminal(rec)) {
    return { ok: true, terminal: true };
  }
  const from_state = rec.state;
  try {
    await step(iii, cfg, rec);
  } catch (err) {
    throw new Error(`transition from ${from_state} failed: ${String(err)}`);
  }
  await persistence.saveRecord(iii, rec);
  return { ok: true, from_state, to_state: rec.state };
}

export function register(iii: ISdk, cfg: TurnOrchestratorConfig): void {
  iii.registerFunction(
    'turn::step',
    async (payload: unknown) => execute(iii, cfg, StepPayloadSchema.parse(payload)),
    {
      description: 'Run one durable state machine transition for a session.',
    },
  );
  iii.registerTrigger({
    type: 'durable:subscriber',
    function_id: 'turn::step',
    config: { topic: 'turn::step_requested' },
  });
}
