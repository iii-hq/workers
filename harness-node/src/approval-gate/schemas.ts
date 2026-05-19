/**
 * Wire schemas + inferred types for the approval gate. Replaces the
 * hand-rolled `types.ts` with Zod-derived equivalents so that boundary
 * validation, type inference, and JSON-schema export all flow from one
 * source. Mirrors the pattern in `src/types/provider.ts`.
 */

import { z } from 'zod';
import { zodToJsonSchema } from 'zod-to-json-schema';

export const FN_RESOLVE = 'approval::resolve';
export const STATE_SCOPE = 'approvals';
export const DENIAL_SCHEMA_VERSION = 1;

export const WireDecisionSchema = z.enum(['allow', 'deny']);
export type WireDecision = z.infer<typeof WireDecisionSchema>;

/**
 * Payload for the `state::set` engine function when storing an approval
 * decision under the approvals scope. Typed so callers in this module get
 * compile-time shape checking on their literal payload construction.
 */
export type StateSetApprovalPayload = {
  scope: string;
  key: string;
  value: { decision: WireDecision; reason: string | null };
};

/** Payload for `turn::step` (the orchestrator wake-up function). */
export type TurnStepPayload = { session_id: string };

export const DeniedBySchema = z.enum(['permissions', 'user', 'gate_unavailable']);
export type DeniedBy = z.infer<typeof DeniedBySchema>;

export const MatchedConstraintSchema = z.object({
  field: z.string(),
  operator: z.string(),
  value: z.unknown(),
});
export type MatchedConstraint = z.infer<typeof MatchedConstraintSchema>;

export const DenialEnvelopeSchema = z.object({
  schema_version: z.literal(DENIAL_SCHEMA_VERSION),
  status: z.literal('denied'),
  denied_by: DeniedBySchema,
  function_id: z.string(),
  rule_id: z.string().optional(),
  rule_action: z.literal('deny').optional(),
  matched_constraint: MatchedConstraintSchema.optional(),
  args_excerpt: z.unknown().optional(),
  reason: z.string(),
});
export type DenialEnvelope = z.infer<typeof DenialEnvelopeSchema>;

/**
 * Wire payload for `approval::resolve`. The schema accepts either
 * `function_call_id` or the legacy `tool_call_id` alias, and emits a
 * normalized object with a single non-optional `function_call_id` so
 * callers don't have to fall back / non-null-assert.
 */
export const ResolvePayloadSchema = z
  .object({
    session_id: z.string().min(1),
    function_call_id: z.string().min(1).optional(),
    tool_call_id: z.string().min(1).optional(),
    decision: WireDecisionSchema,
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
    return {
      session_id: v.session_id,
      function_call_id: fnId,
      decision: v.decision,
      reason: v.reason ?? null,
    };
  });
export type ResolvePayload = z.infer<typeof ResolvePayloadSchema>;

/**
 * Decoded reply from `policy::check_permissions`. The discriminated union
 * gives callers exhaustive narrowing without manual `typeof` chains.
 */
export const PolicyReplySchema = z.discriminatedUnion('decision', [
  z.object({
    decision: z.literal('allow'),
    rule_id: z.string().optional().default(''),
  }),
  z.object({
    decision: z.literal('deny'),
    rule_id: z.string().optional().default(''),
    matched_constraint: MatchedConstraintSchema.nullable().default(null),
  }),
]);
export type PolicyReply = z.infer<typeof PolicyReplySchema>;

/**
 * Outcome shape consumed by the orchestrator hook. The wire reply
 * (`PolicyReply`) covers `allow` / `deny`; we add the `needs_approval`
 * fallback for unparseable / unknown replies.
 */
export type PolicyOutcome = PolicyReply | { decision: 'needs_approval' };

/**
 * Decode the wire reply from `policy::check_permissions`. Anything the
 * schema can't validate falls through to `needs_approval` so the
 * orchestrator pauses for human review (safe default).
 */
export function parsePolicyReply(value: unknown): PolicyOutcome {
  const parsed = PolicyReplySchema.safeParse(value);
  return parsed.success ? parsed.data : { decision: 'needs_approval' };
}

/**
 * State-trigger envelope delivered by the iii engine. Owned by the engine,
 * not us — we only consume it. Captures the minimum fields the gate uses.
 */
export const StateEventSchema = z.object({
  event_type: z.enum(['state:created', 'state:updated', 'state:deleted']),
  key: z.string(),
  new_value: z.unknown().optional(),
});
export type StateEvent = z.infer<typeof StateEventSchema>;

/**
 * Narrower variant used by `isDecisionWrite`. Matches only the state events
 * that look like an actual decision write: created / updated (NOT deleted),
 * with a `new_value` object carrying a `decision` string. `passthrough()`
 * keeps the other fields the engine might add later.
 */
export const DecisionWriteEventSchema = StateEventSchema.extend({
  event_type: z.enum(['state:created', 'state:updated']),
  new_value: z.object({ decision: z.string() }).passthrough(),
});

/**
 * Composite key used in the `approvals` state scope:
 * `<session_id>/<function_call_id>`. Neither half may contain `/`.
 */
export function pendingKey(session_id: string, function_call_id: string): string {
  if (session_id.includes('/')) throw new Error('session_id must not contain "/"');
  if (function_call_id.includes('/')) {
    throw new Error('function_call_id must not contain "/"');
  }
  return `${session_id}/${function_call_id}`;
}

/**
 * Tolerant split: the first `/` separates the session id from the function
 * call id. The function_call_id half may itself contain `/` and is
 * preserved verbatim, so this is forward-compatible if the id format ever
 * changes to include slashes. Returns `null` for keys with no `/` or an
 * empty leading session id.
 */
export function parsePendingKey(
  key: string,
): { session_id: string; function_call_id: string } | null {
  const slash = key.indexOf('/');
  if (slash <= 0) return null;
  return {
    session_id: key.slice(0, slash),
    function_call_id: key.slice(slash + 1),
  };
}

/**
 * Derived JSON schema for the `approval::resolve` request payload. Passed
 * to `iii.registerFunction(..., { request_format })` so the engine's
 * directory exposes a typed signature. No equivalent for the trigger /
 * condition fns — their payload shape is engine-owned.
 */
export const ResolvePayloadJsonSchema = zodToJsonSchema(ResolvePayloadSchema, {
  name: 'ResolvePayload',
});
