/**
 * Read approval decisions, compute resume or park outcome, and register the FSM step.
 */

import type { ISdk } from '../../runtime/iii.js';
import { text } from '../../types/content.js';
import type { FunctionResult } from '../../types/function.js';
import type { PreparedCall } from '../function-execute/types.js';
import { runTransition } from '../run-transition.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';
import { transitionTo, type AwaitingApprovalEntry, type TurnStateRecord } from '../state.js';
import {
  createAwaitingApprovalPorts,
  type ApprovalDecision,
  type AwaitingApprovalOutcome,
  type AwaitingApprovalPorts,
} from './ports.js';

export function denialResultFromDecision(decision: ApprovalDecision): FunctionResult {
  const reason =
    decision.reason ?? (decision.decision === 'aborted' ? 'session_aborted' : 'denied');
  const message =
    decision.decision === 'aborted'
      ? `Function call aborted: ${reason}`
      : `Permission denied by user: ${reason}`;
  return {
    content: [text(message)],
    details: {
      approval_denied: true,
      decision: decision.decision,
      reason,
    },
    terminate: false,
  };
}

export function applyDecisionToPrepared(
  current: PreparedCall,
  decision: ApprovalDecision,
): PreparedCall {
  if (decision.decision === 'allow') {
    return { route: 'pre_approved', call: current.call };
  }
  return {
    route: 'synthetic',
    call: current.call,
    result: denialResultFromDecision(decision),
  };
}

export function foldDecisionsIntoPrepared(
  prepared: readonly PreparedCall[],
  awaiting: AwaitingApprovalEntry[],
  decisions: ApprovalDecision[],
): PreparedCall[] {
  const next = [...prepared];
  for (let i = 0; i < awaiting.length; i++) {
    const entry = awaiting[i];
    const decision = decisions[i];
    if (!entry || !decision) continue;
    const idx = next.findIndex((pe) => pe.call.id === entry.function_call_id);
    if (idx < 0) continue;
    const current = next[idx];
    if (!current) continue;
    next[idx] = applyDecisionToPrepared(current, decision);
  }
  return next;
}

export async function processAwaitingApproval(
  ports: AwaitingApprovalPorts,
  rec: TurnStateRecord,
): Promise<AwaitingApprovalOutcome> {
  const awaiting = rec.awaiting_approval ?? [];
  if (awaiting.length === 0) {
    return { kind: 'resume_empty' };
  }

  const decisions = await Promise.all(
    awaiting.map((entry) => ports.readDecision(rec.session_id, entry.function_call_id)),
  );

  if (decisions.some((decision) => decision === null)) {
    return { kind: 'parked' };
  }

  const prepared = foldDecisionsIntoPrepared(
    rec.work?.prepared ?? [],
    awaiting,
    decisions as NonNullable<(typeof decisions)[number]>[],
  );

  return { kind: 'resume', prepared };
}

export function applyAwaitingApprovalOutcome(
  rec: TurnStateRecord,
  outcome: AwaitingApprovalOutcome,
): void {
  if (outcome.kind === 'parked') {
    return;
  }

  if (outcome.kind === 'resume' && rec.work) {
    rec.work = { ...rec.work, prepared: outcome.prepared };
  }

  rec.awaiting_approval = [];
  transitionTo(rec, 'function_execute');
}

export async function runAwaitingApproval(
  ports: AwaitingApprovalPorts,
  rec: TurnStateRecord,
): Promise<void> {
  const outcome = await processAwaitingApproval(ports, rec);
  applyAwaitingApprovalOutcome(rec, outcome);
}

export async function handleAwaitingApproval(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const ports = createAwaitingApprovalPorts(iii);
  await runAwaitingApproval(ports, rec);
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::function_awaiting_approval',
    async (payload: TurnStepPayload) => {
      const parsed = TurnStepPayloadSchema.parse(payload);
      return runTransition(iii, 'function_awaiting_approval', handleAwaitingApproval, parsed);
    },
    {
      description:
        'Run one durable FSM transition for session in state function_awaiting_approval: read approval decisions and resume.',
    },
  );
}
