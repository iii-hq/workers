/**
 * `turn::finishing` — the terminal step.
 *
 * A turn whose work is durably committed transitions to `finishing` (see
 * {@link transitionToFinishing}). This step emits the single `agent_end`
 * run-end signal and advances to `stopped`. Keeping the terminal signal in its
 * own step means a crash BEFORE the work was saved re-runs the work without
 * consumers ever having seen a premature run-end; a crash IN this step simply
 * re-emits agent_end (idempotent-tolerated) on replay.
 */

import type { ISdk } from '../runtime/iii.js';
import { runTransition } from './run-transition.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from './schemas.js';
import { createTurnStatePorts } from './state-runtime/ports.js';
import type { TurnStateRecord } from './state.js';

export async function handleFinishing(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const ports = createTurnStatePorts(iii);
  await ports.finishSession(rec);
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::finishing',
    async (payload: TurnStepPayload) => {
      const parsed = TurnStepPayloadSchema.parse(payload);
      return runTransition(iii, 'finishing', (i, rec) => handleFinishing(i, rec), parsed);
    },
    {
      description:
        'Run one durable FSM transition for session in state finishing: emit agent_end and advance to stopped.',
    },
  );
}
