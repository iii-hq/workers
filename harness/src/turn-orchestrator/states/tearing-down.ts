import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { emit } from '../events.js';
import * as persistence from '../persistence.js';
import { type TurnStateRecord, transitionTo } from '../state.js';

export async function handleTearingDown(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const sandbox_id = await persistence.loadSandboxId(iii, rec.session_id);
  if (sandbox_id) {
    try {
      await iii.trigger<unknown, unknown>({
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
  const messages = await persistence.loadMessages(iii, rec.session_id);
  await emit(iii, rec.session_id, { type: 'agent_end', messages });
  transitionTo(rec, 'stopped');
}
// reload 1779112003
