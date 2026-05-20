/**
 * Wire schemas + inferred types for the approval gate. Zod validates
 * ingress payloads; exported types cover denial envelopes and shared
 * orchestrator contracts.
 */

import type { RegisterFunctionOptions } from 'iii-sdk';
import { z } from 'zod';
import { zodToJsonSchema } from 'zod-to-json-schema';

export const STATE_SCOPE = 'approvals';
export const DENIAL_SCHEMA_VERSION = 1;

const wireDecisionSchema = z.enum(['allow', 'deny']);

const deniedBySchema = z.enum(['permissions', 'user', 'gate_unavailable']);

const matchedConstraintSchema = z.object({
  field: z.string(),
  operator: z.string(),
  value: z.unknown(),
});
export type MatchedConstraint = z.infer<typeof matchedConstraintSchema>;

const denialEnvelopeSchema = z.object({
  schema_version: z.literal(DENIAL_SCHEMA_VERSION),
  status: z.literal('denied'),
  denied_by: deniedBySchema,
  function_id: z.string(),
  rule_id: z.string().optional(),
  rule_action: z.literal('deny').optional(),
  matched_constraint: matchedConstraintSchema.optional(),
  args_excerpt: z.unknown().optional(),
  reason: z.string(),
});
export type DenialEnvelope = z.infer<typeof denialEnvelopeSchema>;

/**
 * Wire payload for `approval::resolve`. Accepts `function_call_id` or the
 * legacy `tool_call_id` alias; output always has `function_call_id` set.
 * Rejects "/" in either id at the boundary — it is the reserved separator in
 * the state key, so a slashed id is refused here rather than thrown on later.
 */
export const ResolvePayloadSchema = z
  .object({
    session_id: z.string().min(1),
    function_call_id: z.string().min(1).optional(),
    tool_call_id: z.string().min(1).optional(),
    decision: wireDecisionSchema,
    reason: z.string().nullable().optional(),
  })
  .transform((v, ctx) => {
    const fnId = v.function_call_id ?? v.tool_call_id;
    if (!fnId) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ['function_call_id'],
        message: 'function_call_id or tool_call_id is required',
      });
      return z.NEVER;
    }
    if (v.session_id.includes('/')) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ['session_id'],
        message: 'session_id must not contain "/"',
      });
      return z.NEVER;
    }
    if (fnId.includes('/')) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ['function_call_id'],
        message: 'function_call_id must not contain "/"',
      });
      return z.NEVER;
    }
    return {
      session_id: v.session_id,
      function_call_id: fnId,
      decision: v.decision,
      reason: v.reason ?? null,
    };
  });
export type ResolvePayloadInput = z.input<typeof ResolvePayloadSchema>;

const policyReplySchema = z.discriminatedUnion('decision', [
  z.object({
    decision: z.literal('allow'),
    rule_id: z.string().optional().default(''),
  }),
  z.object({
    decision: z.literal('deny'),
    rule_id: z.string().optional().default(''),
    matched_constraint: matchedConstraintSchema.nullable().default(null),
  }),
]);

type PolicyReply = z.infer<typeof policyReplySchema>;
type PolicyOutcome = PolicyReply | { decision: 'needs_approval' };

/** Decode `policy::check_permissions` reply; unknown shapes → `needs_approval`. */
export function parsePolicyReply(value: unknown): PolicyOutcome {
  const parsed = policyReplySchema.safeParse(value);
  return parsed.success ? parsed.data : { decision: 'needs_approval' };
}

/** State key in scope `approvals`: `<session_id>/<function_call_id>`. */
export function pendingKey(session_id: string, function_call_id: string): string {
  if (session_id.includes('/')) throw new Error('session_id must not contain "/"');
  if (function_call_id.includes('/')) {
    throw new Error('function_call_id must not contain "/"');
  }
  return `${session_id}/${function_call_id}`;
}

const approvalDecisionSchema = z.enum(['allow', 'deny', 'aborted']);

export const ApprovalResumePayloadSchema = z.object({
  decision: approvalDecisionSchema,
  reason: z.string().nullable(),
});

export function approvalResumeFnId(session_id: string, function_call_id: string): string {
  return `turn::approval_resume::${pendingKey(session_id, function_call_id)}`;
}

export const resolveFunctionOptions = {
  description:
    'Flip an approval to allow or deny. Invokes the per-call resume function to persist and wake the turn.',
  request_format: zodToJsonSchema(ResolvePayloadSchema, { name: 'ResolvePayload' }),
} as RegisterFunctionOptions;
