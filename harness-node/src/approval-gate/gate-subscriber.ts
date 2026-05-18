/**
 * `policy::approval_gate` durable subscriber. Mirrors the body of
 * `approval-gate/src/lib.rs::register::subscriber_fn` post-PR #150.
 *
 * Flow:
 *  1. extract the incoming call from the durable envelope
 *  2. consult policy (allow / deny / needs_approval). Allow and Deny both
 *     return immediately with a marked block reply (subscriber +
 *     approval_gate flags so the orchestrator's `publishFailureFromResponse`
 *     can detect that the gate actually replied — fail-closed).
 *  3. needs_approval: delegate to `handleIntercept`, which writes the
 *     pending record (fail-closed on state-bus failure) and returns a
 *     pending/denied block envelope. We additionally emit the
 *     `approval_requested` lifecycle event when the entry reaches pending.
 *
 * Note: `approval_resolved` and `approval_wake_failed` events are emitted
 * by `handleResolveWithEvents` (in `pending.ts`) when the human resolves,
 * not from this subscriber. This subscriber returns synchronously without
 * waiting for the human decision.
 */

import { uuidLike } from '../runtime/ids.js';
import type { ISdk } from '../runtime/iii.js';
import { streamSet } from '../runtime/stream.js';
import type { ApprovalGateConfig } from './config.js';
import { permissionsDenyEnvelope } from './denial.js';
import { emitApprovalRequested } from './events.js';
import { handleIntercept } from './intercept.js';
import { type PolicyOutcome, consultPolicy } from './policy-consult.js';
import type { StateBus } from './state-bus.js';
import { type IncomingCall, SUBSCRIBER_NAME, blockReplyFor, extractCall } from './types.js';

async function writeHookReply(
  iii: ISdk,
  stream_name: string,
  event_id: string,
  reply: unknown,
): Promise<void> {
  if (!stream_name || !event_id) return;
  await streamSet(iii, {
    stream_name,
    group_id: event_id,
    item_id: uuidLike(),
    data: reply,
  });
}

export type GateHandlerContext = {
  iii: ISdk;
  bus: StateBus;
  cfg: ApprovalGateConfig;
};

export async function handleGateEvent(
  ctx: GateHandlerContext,
  envelope: unknown,
): Promise<unknown> {
  const call = extractCall(envelope);
  if (!call) return { block: false };

  // Step 1: consult policy.
  const outcome = await consultPolicyOrFallback(ctx.iii, ctx.cfg, call);

  if (outcome.kind === 'allow') {
    const reply = blockReplyFor({ kind: 'allow' });
    await writeHookReply(ctx.iii, call.reply_stream, call.event_id, reply);
    return reply;
  }

  if (outcome.kind === 'deny') {
    const env = permissionsDenyEnvelope(
      call.function_id,
      outcome.rule_id,
      outcome.matched_constraint,
      call.args,
    );
    const reply = {
      block: true,
      reason: env.reason,
      denial: env,
      subscriber: SUBSCRIBER_NAME,
      approval_gate: true,
    };
    await streamSet(ctx.iii, {
      stream_name: 'agent::events',
      group_id: call.session_id,
      item_id: `approval-${uuidLike()}`,
      data: {
        type: 'function_call_denied',
        function_call_id: call.function_call_id,
        tool_call_id: call.function_call_id,
        function_id: call.function_id,
        tool_name: call.function_id,
        denial: env,
      },
    });
    await writeHookReply(ctx.iii, call.reply_stream, call.event_id, reply);
    return reply;
  }

  // Step 2: pause flow via handleIntercept (fail-closed on state-bus failure).
  // The policy worker already decided `needs_approval`, so we MUST intercept
  // even if the run's approval_required list is empty (e.g. `agent::call`
  // dispatches with []). Synthesize membership so handleIntercept's
  // requiresApproval gate doesn't short-circuit back to allow.
  const callForIntercept: IncomingCall = call.approval_required.includes(call.function_id)
    ? call
    : { ...call, approval_required: [...call.approval_required, call.function_id] };
  const now = Date.now();
  const reply = await handleIntercept(
    ctx.bus,
    ctx.cfg.approval_state_scope,
    callForIntercept,
    now,
    ctx.cfg.default_timeout_ms,
  );
  if ((reply as Record<string, unknown>).status === 'pending') {
    await emitApprovalRequested(ctx.iii, call.session_id, {
      function_call_id: call.function_call_id,
      function_id: call.function_id,
      args: call.args,
      expires_at: now + ctx.cfg.default_timeout_ms,
    });
  }
  await writeHookReply(ctx.iii, call.reply_stream, call.event_id, reply);
  return reply;
}

async function consultPolicyOrFallback(
  iii: ISdk,
  cfg: ApprovalGateConfig,
  call: IncomingCall,
): Promise<PolicyOutcome> {
  const o = await consultPolicy(iii, cfg.policy_function_id, call.function_id, call.args);
  if (o !== null) return o;
  // Legacy fallback: synthesize from approval_required.
  if (call.approval_required.includes(call.function_id)) {
    return { kind: 'needs_approval' };
  }
  return { kind: 'allow', rule_id: 'legacy/approval_required' };
}
