/**
 * `tearing_down` — stop sandbox, emit `agent_end`, transition to `stopped`.
 *
 * **Incoming**: `TurnStateRecord` with `state: 'tearing_down'` from `step()`
 * when the FSM enters teardown (abort, max turns, normal end-turn, etc.).
 * **Outgoing**: mutates `rec` via `transitionTo(rec, 'stopped')`; emits
 * `agent_end` with session messages; calls `sandbox::stop` when a sandbox id
 * exists (best-effort, logs on failure).
 */

import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import type { AgentMessage } from '../../types/agent-message.js';
import { emit } from '../events.js';
import * as persistence from '../persistence.js';
import { type TurnStateRecord, transitionTo } from '../state.js';

type SandboxStopPayload = { sandbox_id: string; wait: true };

export async function handleTearingDown(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const sandbox_id = await persistence.loadSandboxId(iii, rec.session_id);
  if (sandbox_id) {
    try {
      await iii.trigger<SandboxStopPayload, unknown>({
        function_id: 'sandbox::stop',
        payload: { sandbox_id, wait: true },
        timeoutMs: 60_000,
      });
    } catch (err) {
      logger.warn('sandbox::stop failed during teardown', {
        sandbox_id,
        err: String(err),
      });
    }
  }
  const messages: AgentMessage[] = await persistence.loadMessages(iii, rec.session_id);
  await emit(iii, rec.session_id, { type: 'agent_end', messages });
  transitionTo(rec, 'stopped');
}
