/**
 * `policy::approval_gate` durable subscriber. Mirrors the body of
 * `approval-gate/src/lib.rs::register::subscriber_fn`.
 */

import { uuidLike } from '../runtime/ids.js';
import type { ISdk } from '../runtime/iii.js';
import { streamSet } from '../runtime/stream.js';
import type { ApprovalGateConfig } from './config.js';
import { permissionsDenyEnvelope, userDenyEnvelope } from './denial.js';
import { awaitDecision } from './pending.js';
import { type PolicyOutcome, consultPolicy } from './policy-consult.js';
import type { StateBus } from './state-bus.js';
import {
  type DenialEnvelope,
  type IncomingCall,
  buildPendingRecord,
  extractCall,
  pendingKey,
} from './types.js';

const HOOK_REPLY_STREAM = 'agent::hook_reply';

async function writeEvent(iii: ISdk, session_id: string, event: unknown): Promise<void> {
  await streamSet(iii, {
    stream_name: 'agent::events',
    group_id: session_id,
    item_id: `approval-${uuidLike()}`,
    data: event,
  });
}

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

function blockReplyForAllow(): { block: false } {
  return { block: false };
}

function blockReplyForDeny(envelope: DenialEnvelope): { block: true; denial: DenialEnvelope } {
  return { block: true, denial: envelope };
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
    const reply = blockReplyForAllow();
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
    await writeEvent(ctx.iii, call.session_id, {
      type: 'function_call_denied',
      function_call_id: call.function_call_id,
      tool_call_id: call.function_call_id,
      function_id: call.function_id,
      tool_name: call.function_id,
      denial: env,
    });
    const reply = blockReplyForDeny(env);
    await writeHookReply(ctx.iii, call.reply_stream, call.event_id, reply);
    return reply;
  }

  // Step 2: pause-and-wait flow.
  const now = Date.now();
  const expires_at = now + ctx.cfg.default_timeout_ms;
  const record = buildPendingRecord(
    call.function_call_id,
    call.function_id,
    call.args,
    now,
    ctx.cfg.default_timeout_ms,
  );
  try {
    await ctx.bus.set(
      ctx.cfg.approval_state_scope,
      pendingKey(call.session_id, call.function_call_id),
      record,
    );
  } catch {
    const reply = blockReplyForAllow();
    await writeHookReply(ctx.iii, call.reply_stream, call.event_id, reply);
    return reply;
  }
  await writeEvent(ctx.iii, call.session_id, {
    type: 'approval_requested',
    function_call_id: call.function_call_id,
    tool_call_id: call.function_call_id,
    function_id: call.function_id,
    tool_name: call.function_id,
    args: call.args,
    expires_at,
  });
  const awaited = await awaitDecision(
    ctx.bus,
    ctx.cfg.approval_state_scope,
    call.session_id,
    call.function_call_id,
    expires_at,
  );
  let reply: { block: false } | { block: true; denial: DenialEnvelope };
  let decisionStr: 'allow' | 'deny';
  let reasonForEvent: string | null;
  if (awaited.kind === 'allow') {
    reply = blockReplyForAllow();
    decisionStr = 'allow';
    reasonForEvent = null;
  } else {
    const env = userDenyEnvelope(call.function_id, awaited.reason, call.args);
    reply = blockReplyForDeny(env);
    decisionStr = 'deny';
    reasonForEvent = awaited.reason;
  }
  await writeEvent(ctx.iii, call.session_id, {
    type: 'approval_resolved',
    function_call_id: call.function_call_id,
    tool_call_id: call.function_call_id,
    decision: decisionStr,
    reason: reasonForEvent,
  });
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
