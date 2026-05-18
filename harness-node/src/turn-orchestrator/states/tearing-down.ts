import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { emit } from '../events.js';
import * as persistence from '../persistence.js';
import { type TurnStateRecord, transitionTo } from '../state.js';
import { consumeResolvedApprovalEntries } from './functions.js';

export async function handleTearingDown(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  // PR #150: before tearing down, check whether any approvals were
  // resolved just as the turn went terminal. If so, resurrect the turn
  // back into function_execute instead of stopping the session — the
  // resolved approvals complete the tool calls that paused us. Always
  // call consume; it returns `entries: []` cheaply when nothing's
  // resolved. (We used to gate on `approvalRequiredEnabled(request)` but
  // console/web intentionally sends `approval_required: []` — policy
  // owns the decision via iii-permissions.yaml.)
  try {
    const prepared = await consumeResolvedApprovalEntries(iii, rec.session_id);
    if (prepared.length > 0) {
      await persistence.saveExecutedCalls(iii, rec.session_id, []);
      await persistence.savePreparedCalls(iii, rec.session_id, prepared);
      transitionTo(rec, 'function_execute');
      return;
    }
  } catch (err) {
    logger.warn('approval::consume failed during teardown; stopping session', {
      session_id: rec.session_id,
      err: String(err),
    });
  }

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
