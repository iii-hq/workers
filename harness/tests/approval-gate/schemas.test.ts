/**
 * Adversarial tests for the wire schemas. Two properties matter most:
 *   1. Ingress validation refuses ids that would corrupt the state key.
 *   2. parsePolicyReply is fail-CLOSED — anything it can't positively
 *      decode as a clean allow/deny becomes needs_approval (a human gate).
 *      A corrupted or attacker-shaped reply must never decode as `allow`.
 */
import { describe, expect, it } from 'vitest';
import {
  ApprovalDecisionSchema,
  ApprovalResumePayloadSchema,
  ResolvePayloadSchema,
  parsePolicyReply,
  pendingKey,
  resolveFunctionOptions,
} from '../../src/approval-gate/schemas.js';

describe('ResolvePayloadSchema — id normalization & validation', () => {
  it('coerces an omitted reason to null', () => {
    const parsed = ResolvePayloadSchema.parse({
      session_id: 's',
      function_call_id: 'fc-1',
      decision: 'deny',
    });
    expect(parsed.reason).toBeNull();
    expect(parsed.function_call_id).toBe('fc-1');
  });

  it.each([
    ['function_call_id missing', { session_id: 's', decision: 'allow' }],
    ['tool_call_id only (legacy)', { session_id: 's', tool_call_id: 'legacy', decision: 'allow' }],
    ['empty function_call_id', { session_id: 's', function_call_id: '', decision: 'allow' }],
    ['empty session_id', { session_id: '', function_call_id: 'fc', decision: 'allow' }],
    ['slash in session_id', { session_id: 'a/b', function_call_id: 'fc', decision: 'allow' }],
    ['slash in function_call_id', { session_id: 's', function_call_id: 'a/b', decision: 'allow' }],
    ['non-enum decision', { session_id: 's', function_call_id: 'fc', decision: 'maybe' }],
    ['numeric reason', { session_id: 's', function_call_id: 'fc', decision: 'allow', reason: 7 }],
  ])('rejects %s', (_label, payload) => {
    expect(ResolvePayloadSchema.safeParse(payload).success).toBe(false);
  });
});

describe('parsePolicyReply — fail closed', () => {
  it('decodes a well-formed allow and deny', () => {
    expect(parsePolicyReply({ decision: 'allow', rule_id: 'r1' })).toEqual({
      decision: 'allow',
      rule_id: 'r1',
    });
    const deny = parsePolicyReply({
      decision: 'deny',
      rule_id: 'r2',
      matched_constraint: { field: 'cmd', operator: 'eq', value: 'x' },
    });
    expect(deny.decision).toBe('deny');
  });

  it('defaults a deny with no matched_constraint to null rather than failing', () => {
    expect(parsePolicyReply({ decision: 'deny' })).toEqual({
      decision: 'deny',
      rule_id: '',
      matched_constraint: null,
    });
  });

  it('strips unknown fields — extra keys cannot ride along', () => {
    expect(parsePolicyReply({ decision: 'allow', rule_id: 'r', injected: 'x' })).toEqual({
      decision: 'allow',
      rule_id: 'r',
    });
  });

  it.each([
    ['empty object', {}],
    ['null', null],
    ['array', []],
    ['string', 'allow'],
    ['unknown decision', { decision: 'maybe' }],
    ['allow with wrong-typed rule_id', { decision: 'allow', rule_id: 123 }],
    [
      'deny with malformed matched_constraint',
      { decision: 'deny', matched_constraint: { field: 1 } },
    ],
  ])('falls through to needs_approval for %s', (_label, input) => {
    expect(parsePolicyReply(input)).toEqual({ decision: 'needs_approval' });
  });
});

describe('state-key derivation — separator integrity', () => {
  it('derives <session>/<fcall>', () => {
    expect(pendingKey('sess-1', 'fc-1')).toBe('sess-1/fc-1');
  });

  it.each([
    ['session', 'a/b', 'c'],
    ['function_call', 'a', 'b/c'],
  ])('throws if the %s id smuggles a slash', (_which, session, fcall) => {
    expect(() => pendingKey(session, fcall)).toThrow();
  });
});

describe('ApprovalDecisionSchema', () => {
  it('accepts the three terminal decisions with an explicit reason', () => {
    for (const decision of ['allow', 'deny', 'aborted'] as const) {
      expect(ApprovalDecisionSchema.parse({ decision, reason: null })).toEqual({
        decision,
        reason: null,
      });
    }
  });

  it('rejects a missing reason and an unknown decision', () => {
    expect(ApprovalDecisionSchema.safeParse({ decision: 'allow' }).success).toBe(false);
    expect(ApprovalDecisionSchema.safeParse({ decision: 'paused', reason: null }).success).toBe(
      false,
    );
  });

  it('keeps ApprovalResumePayloadSchema as a deprecated alias', () => {
    expect(ApprovalResumePayloadSchema).toBe(ApprovalDecisionSchema);
  });
});

describe('resolveFunctionOptions', () => {
  it('carries a JSON-serializable request_format for iii registration', () => {
    expect(() => JSON.stringify(resolveFunctionOptions.request_format)).not.toThrow();
  });
});
