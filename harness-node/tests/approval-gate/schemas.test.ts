import { describe, expect, it } from 'vitest';
import {
  DENIAL_SCHEMA_VERSION,
  FN_RESOLVE,
  PolicyReplySchema,
  ResolvePayloadJsonSchema,
  ResolvePayloadSchema,
  STATE_SCOPE,
  StateEventSchema,
  WireDecisionSchema,
  parsePendingKey,
  parsePolicyReply,
  pendingKey,
} from '../../src/approval-gate/schemas.js';

describe('approval-gate constants', () => {
  it('exposes stable wire ids', () => {
    expect(FN_RESOLVE).toBe('approval::resolve');
    expect(STATE_SCOPE).toBe('approvals');
    expect(DENIAL_SCHEMA_VERSION).toBe(1);
  });
});

describe('WireDecisionSchema', () => {
  it('accepts allow / deny', () => {
    expect(WireDecisionSchema.parse('allow')).toBe('allow');
    expect(WireDecisionSchema.parse('deny')).toBe('deny');
  });
  it('rejects anything else', () => {
    expect(() => WireDecisionSchema.parse('needs_approval')).toThrow();
    expect(() => WireDecisionSchema.parse('')).toThrow();
  });
});

describe('ResolvePayloadSchema.transform', () => {
  it('normalizes function_call_id to non-optional on the happy path', () => {
    const parsed = ResolvePayloadSchema.parse({
      session_id: 'sess-1',
      function_call_id: 'call-1',
      decision: 'allow',
    });
    expect(parsed).toEqual({
      session_id: 'sess-1',
      function_call_id: 'call-1',
      decision: 'allow',
      reason: null,
    });
  });

  it('accepts tool_call_id as a fallback and emits it as function_call_id', () => {
    const parsed = ResolvePayloadSchema.parse({
      session_id: 'sess-1',
      tool_call_id: 'legacy-call-1',
      decision: 'deny',
      reason: 'nope',
    });
    expect(parsed).toEqual({
      session_id: 'sess-1',
      function_call_id: 'legacy-call-1',
      decision: 'deny',
      reason: 'nope',
    });
  });

  it('normalizes missing / undefined reason to null', () => {
    const parsed = ResolvePayloadSchema.parse({
      session_id: 'sess-1',
      function_call_id: 'call-1',
      decision: 'allow',
    });
    expect(parsed.reason).toBeNull();
  });

  it('fails when both id fields are missing', () => {
    const result = ResolvePayloadSchema.safeParse({
      session_id: 'sess-1',
      decision: 'allow',
    });
    expect(result.success).toBe(false);
    if (!result.success) {
      const paths = result.error.issues.map((i) => i.path.join('.'));
      expect(paths).toContain('function_call_id');
    }
  });

  it('fails on missing session_id', () => {
    const result = ResolvePayloadSchema.safeParse({
      function_call_id: 'call-1',
      decision: 'allow',
    });
    expect(result.success).toBe(false);
    if (!result.success) {
      const paths = result.error.issues.map((i) => i.path.join('.'));
      expect(paths).toContain('session_id');
    }
  });

  it('fails on bad decision enum', () => {
    const result = ResolvePayloadSchema.safeParse({
      session_id: 'sess-1',
      function_call_id: 'call-1',
      decision: 'maybe',
    });
    expect(result.success).toBe(false);
    if (!result.success) {
      const paths = result.error.issues.map((i) => i.path.join('.'));
      expect(paths).toContain('decision');
    }
  });
});

describe('PolicyReplySchema (discriminated union)', () => {
  it('parses allow', () => {
    const parsed = PolicyReplySchema.parse({ decision: 'allow', rule_id: 'r1' });
    expect(parsed.decision).toBe('allow');
    if (parsed.decision === 'allow') {
      expect(parsed.rule_id).toBe('r1');
    }
  });

  it('defaults rule_id to empty string when omitted', () => {
    const parsed = PolicyReplySchema.parse({ decision: 'allow' });
    if (parsed.decision === 'allow') {
      expect(parsed.rule_id).toBe('');
    }
  });

  it('parses deny with matched_constraint', () => {
    const parsed = PolicyReplySchema.parse({
      decision: 'deny',
      rule_id: 'r2',
      matched_constraint: { field: 'cmd', operator: 'startsWith', value: 'rm ' },
    });
    expect(parsed.decision).toBe('deny');
    if (parsed.decision === 'deny') {
      expect(parsed.matched_constraint).toEqual({
        field: 'cmd',
        operator: 'startsWith',
        value: 'rm ',
      });
    }
  });

  it('parses deny without matched_constraint', () => {
    const parsed = PolicyReplySchema.parse({ decision: 'deny', rule_id: 'r3' });
    expect(parsed.decision).toBe('deny');
    if (parsed.decision === 'deny') {
      expect(parsed.matched_constraint ?? null).toBeNull();
    }
  });

  it('rejects unknown decision discriminator', () => {
    expect(() => PolicyReplySchema.parse({ decision: 'needs_approval' })).toThrow();
  });
});

describe('parsePolicyReply', () => {
  it('decodes allow with rule_id', () => {
    expect(parsePolicyReply({ decision: 'allow', rule_id: 'git/status' })).toEqual({
      decision: 'allow',
      rule_id: 'git/status',
    });
  });

  it('decodes deny with constraint', () => {
    const o = parsePolicyReply({
      decision: 'deny',
      rule_id: 'git/no-force-push',
      matched_constraint: { field: 'command', operator: 'matches', value: '^git push --force' },
    });
    expect(o.decision).toBe('deny');
    if (o.decision === 'deny') {
      expect(o.rule_id).toBe('git/no-force-push');
      expect(o.matched_constraint?.field).toBe('command');
    }
  });

  it('decodes deny without a matched_constraint (null default)', () => {
    const o = parsePolicyReply({ decision: 'deny', rule_id: 'r3' });
    expect(o.decision).toBe('deny');
    if (o.decision === 'deny') {
      expect(o.matched_constraint).toBeNull();
    }
  });

  it('falls through to needs_approval for unknown / missing decision', () => {
    expect(parsePolicyReply({})).toEqual({ decision: 'needs_approval' });
    expect(parsePolicyReply({ decision: 'weird' })).toEqual({ decision: 'needs_approval' });
  });

  it('falls through to needs_approval for non-object input', () => {
    expect(parsePolicyReply(null)).toEqual({ decision: 'needs_approval' });
    expect(parsePolicyReply('allow')).toEqual({ decision: 'needs_approval' });
    expect(parsePolicyReply(undefined)).toEqual({ decision: 'needs_approval' });
  });
});

describe('StateEventSchema', () => {
  it('accepts state:created with new_value', () => {
    const parsed = StateEventSchema.parse({
      event_type: 'state:created',
      key: 'sess-1/call-1',
      new_value: { decision: 'allow' },
    });
    expect(parsed.event_type).toBe('state:created');
  });

  it('accepts state:deleted (filter happens downstream)', () => {
    const parsed = StateEventSchema.parse({
      event_type: 'state:deleted',
      key: 'sess-1/call-1',
    });
    expect(parsed.event_type).toBe('state:deleted');
  });

  it('rejects unknown event types', () => {
    const result = StateEventSchema.safeParse({
      event_type: 'state:weird',
      key: 'k',
    });
    expect(result.success).toBe(false);
  });
});

describe('pendingKey', () => {
  it('joins valid ids', () => {
    expect(pendingKey('sess-1', 'call-1')).toBe('sess-1/call-1');
  });
  it('throws if session_id contains "/"', () => {
    expect(() => pendingKey('a/b', 'c')).toThrow();
  });
  it('throws if function_call_id contains "/"', () => {
    expect(() => pendingKey('a', 'b/c')).toThrow();
  });
});

describe('parsePendingKey (tolerant)', () => {
  it('splits on the first slash', () => {
    expect(parsePendingKey('sess-1/call-1')).toEqual({
      session_id: 'sess-1',
      function_call_id: 'call-1',
    });
  });

  // R3 regression guard — a function_call_id with embedded '/' is preserved
  // verbatim. This protects forward compatibility if iii ever changes the id
  // format to include slashes.
  it('preserves slashes in the function_call_id half (R3)', () => {
    expect(parsePendingKey('sess-1/fcid/with/slashes')).toEqual({
      session_id: 'sess-1',
      function_call_id: 'fcid/with/slashes',
    });
  });

  it('returns null when the key has no slash', () => {
    expect(parsePendingKey('no-slash-here')).toBeNull();
  });

  it('returns null on a leading slash (empty session_id)', () => {
    expect(parsePendingKey('/call-1')).toBeNull();
  });

  it('returns null on empty input', () => {
    expect(parsePendingKey('')).toBeNull();
  });
});

describe('ResolvePayloadJsonSchema', () => {
  it('is a JSON schema object suitable for registration', () => {
    expect(ResolvePayloadJsonSchema).toBeTypeOf('object');
    // zod-to-json-schema emits a definitions container when `name` is set;
    // we only need it to be JSON-serializable for the iii directory.
    expect(() => JSON.stringify(ResolvePayloadJsonSchema)).not.toThrow();
  });
});
