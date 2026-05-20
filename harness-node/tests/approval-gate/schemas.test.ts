import { describe, expect, it } from 'vitest';
import {
  ApprovalResumePayloadSchema,
  DENIAL_SCHEMA_VERSION,
  ResolvePayloadSchema,
  STATE_SCOPE,
  approvalResumeFnId,
  parsePolicyReply,
  resolveFunctionOptions,
} from '../../src/approval-gate/schemas.js';

describe('approval-gate constants', () => {
  it('exposes stable wire ids', () => {
    expect(STATE_SCOPE).toBe('approvals');
    expect(DENIAL_SCHEMA_VERSION).toBe(1);
  });
});

describe('ResolvePayloadSchema', () => {
  it('normalizes function_call_id and legacy tool_call_id', () => {
    expect(
      ResolvePayloadSchema.parse({
        session_id: 'sess-1',
        function_call_id: 'call-1',
        decision: 'allow',
      }),
    ).toEqual({
      session_id: 'sess-1',
      function_call_id: 'call-1',
      decision: 'allow',
      reason: null,
    });
    expect(
      ResolvePayloadSchema.parse({
        session_id: 'sess-1',
        tool_call_id: 'legacy-call-1',
        decision: 'deny',
        reason: 'nope',
      }),
    ).toEqual({
      session_id: 'sess-1',
      function_call_id: 'legacy-call-1',
      decision: 'deny',
      reason: 'nope',
    });
  });

  it('rejects missing ids and bad decision', () => {
    expect(ResolvePayloadSchema.safeParse({ session_id: 's', decision: 'allow' }).success).toBe(
      false,
    );
    expect(
      ResolvePayloadSchema.safeParse({
        session_id: 's',
        function_call_id: 'fc',
        decision: 'maybe',
      }).success,
    ).toBe(false);
  });
});

describe('parsePolicyReply', () => {
  it('decodes allow and deny', () => {
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
    if (deny.decision === 'deny') {
      expect(deny.matched_constraint?.field).toBe('cmd');
    }
  });

  it('falls through to needs_approval for unknown input', () => {
    expect(parsePolicyReply({})).toEqual({ decision: 'needs_approval' });
    expect(parsePolicyReply(null)).toEqual({ decision: 'needs_approval' });
  });
});

describe('approvalResumeFnId', () => {
  it('derives turn::approval_resume::<session>/<fcall>', () => {
    expect(approvalResumeFnId('sess-1', 'fc-1')).toBe('turn::approval_resume::sess-1/fc-1');
  });

  it('rejects slashes in ids', () => {
    expect(() => approvalResumeFnId('a/b', 'c')).toThrow();
    expect(() => approvalResumeFnId('a', 'b/c')).toThrow();
  });
});

describe('ApprovalResumePayloadSchema', () => {
  it('accepts allow, deny, aborted', () => {
    expect(ApprovalResumePayloadSchema.parse({ decision: 'allow', reason: null })).toEqual({
      decision: 'allow',
      reason: null,
    });
  });
});

describe('resolveFunctionOptions', () => {
  it('includes a JSON-serializable request_format for iii registration', () => {
    expect(resolveFunctionOptions.request_format).toBeTypeOf('object');
    expect(() => JSON.stringify(resolveFunctionOptions.request_format)).not.toThrow();
  });
});
