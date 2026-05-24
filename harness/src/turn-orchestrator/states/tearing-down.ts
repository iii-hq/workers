/**
 * `turn::tearing_down`. Emit `agent_end` and transition to `stopped`.
 *
 * **Incoming**: flat `{ session_id }` via FIFO enqueue on `turn-step`.
 * **Outgoing**: `{ ok, from_state, to_state }` on success; stale skip when state drifted.
 */

import type { ISdk } from '../../runtime/iii.js';
import type { AgentMessage } from '../../types/agent-message.js';
import { emit } from '../events.js';
import * as persistence from '../persistence.js';
import { runTransition } from '../run-transition.js';
import { type TurnStateRecord, transitionTo } from '../state.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';

export async function handleTearingDown(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const messages: AgentMessage[] = await persistence.loadMessages(iii, rec.session_id);
  await emit(iii, rec.session_id, { type: 'agent_end', messages });
  transitionTo(rec, 'stopped');
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::tearing_down',
    async (payload: TurnStepPayload) => {
      const parsed = TurnStepPayloadSchema.parse(payload);
      return runTransition(iii, 'tearing_down', handleTearingDown, parsed);
    },
    {
      description:
        'Run one durable FSM transition for session in state tearing_down: emit agent_end and mark stopped.',
    },
  );
}
