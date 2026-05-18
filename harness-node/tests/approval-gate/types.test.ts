import { describe, expect, it } from 'vitest';
import { buildPendingRecord, extractCall, pendingKey } from '../../src/approval-gate/types.js';

describe('pendingKey', () => {
  it('joins session and call ids', () => {
    expect(pendingKey('s1', 'tc-1')).toBe('s1/tc-1');
  });
  it('rejects ids that contain "/"', () => {
    expect(() => pendingKey('a/b', 'tc')).toThrow();
    expect(() => pendingKey('a', 'b/c')).toThrow();
  });
});

describe('buildPendingRecord', () => {
  it('sets status pending and computes expires_at', () => {
    const r = buildPendingRecord('tc-1', 'write', { x: 1 }, 1_000_000, 60_000) as Record<
      string,
      unknown
    >;
    expect(r.status).toBe('pending');
    expect(r.function_call_id).toBe('tc-1');
    expect(r.expires_at).toBe(1_060_000);
  });
});

describe('extractCall', () => {
  it('reads modern envelope shape', () => {
    const envelope = {
      event_id: 'evt-1',
      reply_stream: 'rs-1',
      payload: {
        function_call: { id: 'tc-1', function_id: 'write', arguments: { path: '/tmp/x' } },
        approval_required: ['write'],
        session_id: 's1',
      },
    };
    const call = extractCall(envelope);
    expect(call?.session_id).toBe('s1');
    expect(call?.function_id).toBe('write');
    expect(call?.function_call_id).toBe('tc-1');
    expect(call?.approval_required).toContain('write');
  });

  it('accepts legacy tool_call shape with name field', () => {
    const envelope = {
      event_id: 'evt-1',
      reply_stream: 'rs-1',
      payload: {
        tool_call: { id: 'tc-1', name: 'write', arguments: {} },
        approval_required: ['write'],
        session_id: 's1',
      },
    };
    expect(extractCall(envelope)?.function_id).toBe('write');
  });

  it('returns null when function_call missing', () => {
    expect(
      extractCall({
        event_id: 'evt-1',
        reply_stream: 'rs-1',
        payload: { session_id: 's1' },
      }),
    ).toBeNull();
  });
});
