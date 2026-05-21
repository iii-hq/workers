/**
 * Per-call resume functions for parked approvals. Registered when a call
 * enters `function_awaiting_approval`; invoked by `approval::resolve` or
 * abort. Persists to scope `approvals` and wakes `turn::step`.
 */

import {
  ApprovalResumePayloadSchema,
  STATE_SCOPE,
  approvalResumeFnId,
  pendingKey,
} from '../approval-gate/schemas.js';
import type { FunctionRef, ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import {
  parseStateListKeyedEntries,
  parseStateListValues,
  stateGet,
  stateSet,
} from '../runtime/state.js';
import type { TurnStateRecord } from './state.js';
import { STEP_FN_ID } from './subscriber.js';

const resumeRefs = new Map<string, FunctionRef>();
const TURN_STATE_KEY_RE = /^session\/[^/]+\/turn_state$/;

function isTurnStateRecord(value: unknown): value is TurnStateRecord {
  if (!value || typeof value !== 'object') return false;
  const rec = value as Record<string, unknown>;
  return typeof rec.session_id === 'string' && typeof rec.state === 'string';
}

/** Agent-scope turn_state still parked on human approval. */
function pausedApprovalCalls(
  rec: TurnStateRecord,
): { session_id: string; function_call_ids: string[] } | null {
  if (rec.state !== 'function_awaiting_approval') return null;

  const session_id = rec.session_id;
  if (!session_id) return null;

  const function_call_ids = (rec.awaiting_approval ?? [])
    .map((entry) => entry.function_call_id)
    .filter((id) => id.length > 0);

  return function_call_ids.length > 0 ? { session_id, function_call_ids } : null;
}

function hasStoredDecision(value: unknown): boolean {
  if (!value || typeof value !== 'object') return false;
  const decision = (value as Record<string, unknown>).decision;
  return decision === 'allow' || decision === 'deny' || decision === 'aborted';
}

function unregisterApprovalResume(fnId: string): void {
  const ref = resumeRefs.get(fnId);
  if (!ref) return;
  try {
    ref.unregister();
  } catch {}
  resumeRefs.delete(fnId);
}

async function handleApprovalResume(
  iii: ISdk,
  session_id: string,
  function_call_id: string,
  payload: unknown,
): Promise<void> {
  const fnId = approvalResumeFnId(session_id, function_call_id);
  if (!resumeRefs.has(fnId)) {
    return;
  }
  const parsed = ApprovalResumePayloadSchema.safeParse(payload);
  if (!parsed.success) {
    logger.warn('approval resume: malformed payload', {
      fnId,
      err: String(parsed.error.issues[0]?.message ?? 'unknown'),
    });
    return;
  }

  const key = pendingKey(session_id, function_call_id);
  const existing = await stateGet(iii, STATE_SCOPE, key);
  if (!hasStoredDecision(existing)) {
    await stateSet(iii, STATE_SCOPE, key, {
      decision: parsed.data.decision,
      reason: parsed.data.reason,
    });
  }

  try {
    await iii.trigger({ function_id: STEP_FN_ID, payload: { session_id } });
  } catch (err) {
    logger.warn('approval resume: turn::step invoke failed', { session_id, err: String(err) });
  }

  unregisterApprovalResume(fnId);
}

export function registerApprovalResume(
  iii: ISdk,
  session_id: string,
  function_call_id: string,
): FunctionRef {
  const fnId = approvalResumeFnId(session_id, function_call_id);
  const existing = resumeRefs.get(fnId);
  if (existing) return existing;

  const ref = iii.registerFunction(
    fnId,
    async (payload: unknown) => handleApprovalResume(iii, session_id, function_call_id, payload),
    {
      description:
        'Resume a parked approval: persist decision to approvals scope and wake turn::step.',
    },
  );
  resumeRefs.set(fnId, ref);
  return ref;
}

/** Clears in-memory resume refs (unit tests only). */
export function clearApprovalResumeRegistry(): void {
  resumeRefs.clear();
}

/** Turn_state rows from `state::list` on scope agent (not every value in the scope). */
async function listTurnStateRecords(iii: ISdk): Promise<TurnStateRecord[]> {
  try {
    const resp = await iii.trigger<unknown, unknown>({
      function_id: 'state::list',
      payload: { scope: 'agent' },
    });
    const keyed = parseStateListKeyedEntries(resp);
    if (keyed.some((entry) => typeof entry.key === 'string')) {
      return keyed
        .filter((entry) => entry.key && TURN_STATE_KEY_RE.test(entry.key))
        .map((entry) => entry.value)
        .filter(isTurnStateRecord);
    }
    return parseStateListValues<unknown>(resp).filter(isTurnStateRecord);
  } catch (err) {
    logger.warn('approval resume: state::list failed during recovery', { err: String(err) });
    return [];
  }
}

/** Re-register resume fns for sessions still paused on approval after worker restart. */
export async function recoverPendingApprovals(iii: ISdk): Promise<void> {
  const records = await listTurnStateRecords(iii);

  for (const rec of records) {
    const paused = pausedApprovalCalls(rec);
    if (!paused) continue;

    for (const function_call_id of paused.function_call_ids) {
      registerApprovalResume(iii, paused.session_id, function_call_id);
    }
  }
}
