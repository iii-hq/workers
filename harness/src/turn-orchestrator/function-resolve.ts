/**
 * `harness::function::resolve` — the release/deliver surface for held
 * function calls. The approval-gate worker (or any pre_dispatch hook
 * owner) calls it to settle a call its hook parked:
 *
 *   - `action: "execute"` — the call was approved: the awaiting-approval
 *     wake re-runs it through the normal execution path (released calls
 *     do not re-consult the hook chain).
 *   - `action: "deliver"` — answer the call with the given
 *     content/details/is_error WITHOUT executing (user deny, sweep
 *     timeout).
 *
 * The decision is persisted to the `function_resolutions` scope and the
 * parked turn is woken directly on the turn-step queue. Rows are deleted
 * by the wake after consumption — this scope never accumulates.
 *
 * Unknown `{session_id, function_call_id}` (or a stale `turn_id`) returns
 * `{resolved: false}`, never an error: duplicate decisions and sweep
 * races are benign by contract.
 */

import { TriggerAction, type ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { stateDelete, stateGet, stateSet } from '../runtime/state.js';
import type { ContentBlock } from '../types/content.js';
import {
  FunctionResolvePayloadSchema,
  type FunctionResolvePayload,
  type FunctionResolveResult,
} from './schemas.js';
import { createTurnStore, TURN_STEP_QUEUE } from './state-runtime/store.js';
import { FUNCTION_BATCH_STATES, type TurnStateRecord } from './state.js';

/** Pending decisions for parked calls; key `<session_id>/<function_call_id>`. */
export const FUNCTION_RESOLUTIONS_SCOPE = 'function_resolutions';

export function resolutionKey(session_id: string, function_call_id: string): string {
  return `${session_id}/${function_call_id}`;
}

export type FunctionResolution =
  | { turn_id: string; action: 'execute'; resolved_at: number }
  | {
      turn_id: string;
      action: 'deliver';
      is_error: boolean;
      content: ContentBlock[];
      details?: unknown;
      resolved_at: number;
    };

export async function readResolution(
  iii: ISdk,
  session_id: string,
  function_call_id: string,
): Promise<FunctionResolution | null> {
  const raw = await stateGet<FunctionResolution>(
    iii,
    FUNCTION_RESOLUTIONS_SCOPE,
    resolutionKey(session_id, function_call_id),
  );
  if (!raw || typeof raw !== 'object') return null;
  if (raw.action !== 'execute' && raw.action !== 'deliver') return null;
  return raw;
}

export async function deleteResolution(
  iii: ISdk,
  session_id: string,
  function_call_id: string,
): Promise<void> {
  await stateDelete(iii, FUNCTION_RESOLUTIONS_SCOPE, resolutionKey(session_id, function_call_id));
}

function toResolution(payload: FunctionResolvePayload): FunctionResolution {
  if (payload.action === 'execute') {
    return { turn_id: payload.turn_id, action: 'execute', resolved_at: Date.now() };
  }
  return {
    turn_id: payload.turn_id,
    action: 'deliver',
    is_error: payload.is_error ?? false,
    content: (payload.content ?? []) as ContentBlock[],
    details: payload.details,
    resolved_at: Date.now(),
  };
}

/** Enqueue one `turn::function_awaiting_approval` wake on the turn-step queue. */
export async function enqueueAwaitingApprovalWake(iii: ISdk, session_id: string): Promise<boolean> {
  try {
    await iii.trigger({
      function_id: 'turn::function_awaiting_approval',
      payload: { session_id },
      action: TriggerAction.Enqueue({ queue: TURN_STEP_QUEUE }),
    });
    return true;
  } catch (err) {
    logger.error('function::resolve wake failed; turn stays parked until another resolve', {
      session_id,
      err: String(err),
    });
    return false;
  }
}

/** True when the record currently holds this call awaiting a decision. */
function isHeld(rec: TurnStateRecord, payload: FunctionResolvePayload): boolean {
  if (rec.turn_id !== payload.turn_id) return false;
  if (!(FUNCTION_BATCH_STATES as readonly string[]).includes(rec.state)) return false;
  const awaiting = rec.awaiting_approval ?? [];
  if (!awaiting.some((entry) => entry.function_call_id === payload.function_call_id)) return false;
  // The call must exist in the batch work. A record missing `work` (or the
  // prepared entry) is structurally invalid for a function-batch state —
  // resolving against it would write a row the wake can never consume.
  const prepared = rec.work?.prepared ?? [];
  if (!prepared.some((item) => item.call.id === payload.function_call_id)) return false;
  if (rec.work?.executed?.[payload.function_call_id]) return false;
  return true;
}

export async function handleFunctionResolve(
  iii: ISdk,
  rawPayload: unknown,
): Promise<FunctionResolveResult> {
  const payload = FunctionResolvePayloadSchema.parse(rawPayload);

  // Strict read: a transient state outage must surface as an ERROR (the
  // caller retries; the gate keeps its pending record), never as
  // {resolved: false} — the gate treats that as a benign no-op and
  // deletes its record, which would silently lose the hold.
  const store = createTurnStore(iii);
  const rec = await store.loadRecordStrict(payload.session_id);
  if (!rec || !isHeld(rec, payload)) {
    return { resolved: false };
  }

  const key = resolutionKey(payload.session_id, payload.function_call_id);
  const existing = await readResolution(iii, payload.session_id, payload.function_call_id);
  if (!existing) {
    // First decision wins for SEQUENTIAL duplicates (re-deliveries). Two
    // truly concurrent resolves can both pass this read and last-write-wins
    // on the row — acceptable: both are settled human decisions for the
    // same call, exactly one is consumed, and execution is never doubled
    // (the wake's `executed` guard + per-session lease own that).
    await stateSet(iii, FUNCTION_RESOLUTIONS_SCOPE, key, toResolution(payload));
  }

  const turn_resumed = await enqueueAwaitingApprovalWake(iii, payload.session_id);
  return { resolved: true, turn_resumed };
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'harness::function::resolve',
    async (payload: unknown) => handleFunctionResolve(iii, payload),
    {
      description:
        'Settle one held function call: action "execute" releases it through the normal ' +
        'execution path; action "deliver" answers it with the given content/is_error without ' +
        'executing. Returns {resolved: false} for unknown or already-settled calls. ' +
        '`turn_resumed` means a wake was enqueued, not that the turn already advanced.',
    },
  );
}
