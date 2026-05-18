/**
 * Wire types shared across the gate, the orchestrator hook, and the
 * provider serialisers. Mirrors `approval-gate/src/lib.rs`.
 */

export const FN_RESOLVE = 'approval::resolve';
export const FN_LIST_PENDING = 'approval::list_pending';
export const STATE_SCOPE = 'approvals';
export const DENIAL_SCHEMA_VERSION = 1;

export type DeniedBy = 'permissions' | 'user' | 'gate_unavailable';

export type MatchedConstraint = {
  field: string;
  operator: string;
  value: unknown;
};

export type DenialEnvelope = {
  schema_version: number;
  status: 'denied';
  denied_by: DeniedBy;
  function_id: string;
  rule_id?: string;
  rule_action?: 'deny';
  matched_constraint?: MatchedConstraint;
  args_excerpt?: unknown;
  reason: string;
};

export type WireDecision = 'allow' | 'deny';

export type IncomingCall = {
  session_id: string;
  function_call_id: string;
  function_id: string;
  args: unknown;
  approval_required: string[];
  event_id: string;
  reply_stream: string;
};

export function pendingKey(session_id: string, function_call_id: string): string {
  if (session_id.includes('/')) throw new Error('session_id must not contain "/"');
  if (function_call_id.includes('/')) throw new Error('function_call_id must not contain "/"');
  return `${session_id}/${function_call_id}`;
}

export function buildPendingRecord(
  function_call_id: string,
  function_id: string,
  args: unknown,
  now_ms: number,
  timeout_ms: number,
): Record<string, unknown> {
  return {
    function_call_id,
    function_id,
    args,
    status: 'pending',
    expires_at: now_ms + timeout_ms,
  };
}

export function extractCall(envelope: unknown): IncomingCall | null {
  if (!envelope || typeof envelope !== 'object') return null;
  const obj = envelope as Record<string, unknown>;
  const event_id = typeof obj.event_id === 'string' ? obj.event_id : null;
  const reply_stream = typeof obj.reply_stream === 'string' ? obj.reply_stream : null;
  if (!event_id || !reply_stream) return null;
  const inner =
    obj.payload && typeof obj.payload === 'object' ? (obj.payload as Record<string, unknown>) : obj;
  const session_id = typeof inner.session_id === 'string' ? inner.session_id : null;
  const fc =
    (inner.function_call as Record<string, unknown> | undefined) ??
    (inner.tool_call as Record<string, unknown> | undefined);
  if (!session_id || !fc) return null;
  const function_id =
    (typeof fc.function_id === 'string' && fc.function_id) ||
    (typeof fc.name === 'string' && fc.name) ||
    null;
  const id = typeof fc.id === 'string' ? fc.id : null;
  if (!function_id || !id) return null;
  const args = fc.arguments ?? {};
  const approval_required = Array.isArray(inner.approval_required)
    ? (inner.approval_required as unknown[]).filter((s): s is string => typeof s === 'string')
    : [];
  return {
    session_id,
    function_call_id: id,
    function_id,
    args,
    approval_required,
    event_id,
    reply_stream,
  };
}
