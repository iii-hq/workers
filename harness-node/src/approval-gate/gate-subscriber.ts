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
 *  3. needs_approval: emit `approval_requested` and return a pending block
 *     envelope. The orchestrator owns pending-call state on its turn record.
 *
 * Note: `approval_resolved` events are written by `handleResolveWithEvents`
 * (in `pending.ts`) when the human resolves, not from this subscriber. This
 * subscriber returns synchronously without waiting for the human decision.
 */

import { uuidLike } from '../runtime/ids.js';
import type { ISdk } from '../runtime/iii.js';
import { streamSet } from '../runtime/stream.js';
import type { ApprovalGateConfig } from './config.js';
import { permissionsDenyEnvelope } from './denial.js';
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

  await streamSet(ctx.iii, {
    stream_name: 'agent::events',
    group_id: call.session_id,
    item_id: `approval-${uuidLike()}`,
    data: {
      type: 'approval_requested',
      function_call_id: call.function_call_id,
      tool_call_id: call.function_call_id,
      function_id: call.function_id,
      tool_name: call.function_id,
      args: call.args,
    },
  });
  const reply = {
    block: true,
    status: 'pending' as const,
    subscriber: SUBSCRIBER_NAME,
    approval_gate: true,
  };
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
