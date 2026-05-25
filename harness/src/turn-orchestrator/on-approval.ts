/**
 * Reactive approval wake. A `state` trigger on `scope: 'approvals'` filtered by
 * the `<sid>/<cid>` decision key fires this adapter, which enqueues
 * `turn::{state}` on the durable FIFO queue so the parked session re-reads its
 * decisions in `function_awaiting_approval`. Mirrors `on-abort-signal.ts`.
 *
 * The decision write is produced by `approval::resolve` (approval-gate) or by
 * `abort` — both `state::set` `approvals/<sid>/<cid> = { decision, reason }`.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { listAgentTurnStateRecords } from './persistence.js';
import { ApprovalDecisionEventSchema, type ParsedApprovalDecisionWrite } from './schemas.js';
import { wakeFromRecord } from './wake.js';

export function parseApprovalDecisionWrite(event: unknown): ParsedApprovalDecisionWrite | null {
  const result = ApprovalDecisionEventSchema.safeParse(event);
  return result.success ? result.data : null;
}

export function isApprovalDecisionWrite(event: unknown): boolean {
  return parseApprovalDecisionWrite(event) !== null;
}

export async function execute(iii: ISdk, write: ParsedApprovalDecisionWrite): Promise<void> {
  try {
    await wakeFromRecord(iii, write.session_id);
  } catch (err) {
    logger.warn('turn::on_approval: wake failed', {
      session_id: write.session_id,
      err: String(err),
    });
  }
}

export async function handleApprovalDecisionWrite(iii: ISdk, event: unknown): Promise<void> {
  const write = parseApprovalDecisionWrite(event);
  if (!write) return;
  await execute(iii, write);
}

/** Wake sessions still parked on approval (e.g. a decision arrived during downtime). */
export async function recoverParkedApprovals(iii: ISdk): Promise<void> {
  const records = await listAgentTurnStateRecords(iii);
  for (const rec of records) {
    if (rec.state !== 'function_awaiting_approval') continue;
    try {
      await wakeFromRecord(iii, rec.session_id);
    } catch (err) {
      logger.warn('recoverParkedApprovals: wake failed', {
        session_id: rec.session_id,
        err: String(err),
      });
    }
  }
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::is_approval_decision',
    async (event: unknown) => isApprovalDecisionWrite(event),
    {
      description:
        'Condition: state event writes a decision to approvals/<sid>/<cid> (state:created or state:updated).',
    },
  );

  iii.registerFunction(
    'turn::on_approval',
    async (event: unknown) => handleApprovalDecisionWrite(iii, event),
    {
      description:
        'State trigger adapter on scope=approvals for decision writes; enqueues turn::{state} so the parked session reads its decision.',
    },
  );

  iii.registerTrigger({
    type: 'state',
    function_id: 'turn::on_approval',
    config: {
      scope: 'approvals',
      condition_function_id: 'turn::is_approval_decision',
    },
  });
}
