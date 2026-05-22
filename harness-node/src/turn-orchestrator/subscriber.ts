/**
 * `turn::step` — one FSM transition for a session.
 *
 * **Incoming**: flat `{ session_id }` from durable subscriber (`turn::step_requested`),
 * direct `iii.trigger('turn::step', …)`, and integration tests — same shape.
 * Producers publish via `iii::durable::publish` with `{ topic, data: { session_id } }`;
 * the engine enqueues `data` only.
 *
 * **Outgoing**: `StepResult` with pre/post `TurnState`; throws on missing session
 * (invariant) or transition failure. `turn::should_step` soft-filters unknown/terminal
 * sessions before the durable subscriber invokes `turn::step`.
 */

import type { StateGetInput } from 'iii-sdk/state';
import { z } from 'zod';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { TurnOrchestratorConfig } from './config.js';
import * as persistence from './persistence.js';
import { isTerminal, turnStateKey, type TurnState, type TurnStateRecord } from './state.js';
import { step } from './transitions.js';

export const StepPayloadSchema = z.object({
  session_id: z.string().min(1),
});

export type StepPayload = z.infer<typeof StepPayloadSchema>;

export type StepResult = { ok: true; from_state: TurnState; to_state: TurnState };

export async function shouldStep(iii: ISdk, payload: unknown): Promise<boolean> {
  const parsed = StepPayloadSchema.safeParse(payload);
  if (!parsed.success) return false;
  const rec = await persistence.loadRecord(iii, parsed.data.session_id);
  if (!rec) {
    logger.warn('turn::step for unknown session', { session_id: parsed.data.session_id });
    return false;
  }
  return !isTerminal(rec);
}

export async function execute(
  iii: ISdk,
  cfg: TurnOrchestratorConfig,
  payload: StepPayload,
): Promise<StepResult> {
  const { session_id } = payload;
  const rec = await iii.trigger<StateGetInput, TurnStateRecord>({
    function_id: 'state::get',
    payload: { scope: 'agent', key: turnStateKey(session_id) },
  });
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
  iii.registerFunction('turn::should_step', async (payload: unknown) => shouldStep(iii, payload), {
    description:
      'Condition: durable turn::step_requested payload has a known, non-terminal session.',
  });

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
    config: {
      topic: 'turn::step_requested',
      condition_function_id: 'turn::should_step',
    },
  });
}
