import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import * as persistence from './persistence.js';

const STATE_SCOPE_AGENT = 'agent';
const STATE_SCOPE_APPROVALS = 'approvals';

export async function performAbortSideEffects(iii: ISdk, session_id: string): Promise<void> {
  await trigger(iii, 'state::set', {
    scope: STATE_SCOPE_AGENT,
    key: `session/${session_id}/abort_signal`,
    value: true,
  });

  const rec = await persistence.loadRecord(iii, session_id);
  if (!rec || rec.state !== 'function_awaiting_approval' || !rec.awaiting_approval?.length) {
    return;
  }

  for (const entry of rec.awaiting_approval) {
    await trigger(iii, 'state::set', {
      scope: STATE_SCOPE_APPROVALS,
      key: `${session_id}/${entry.function_call_id}`,
      value: { decision: 'aborted', reason: 'session_aborted' },
    });
  }
}

async function trigger(iii: ISdk, function_id: string, payload: unknown): Promise<void> {
  try {
    await iii.trigger<unknown, unknown>({ function_id, payload });
  } catch (err) {
    logger.warn(`abort side-effect failed: ${function_id}`, { err: String(err) });
  }
}
