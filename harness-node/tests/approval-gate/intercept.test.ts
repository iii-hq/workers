import { describe, expect, it } from 'vitest';
import { handleIntercept } from '../../src/approval-gate/intercept.js';
import { InMemoryStateBus, type StateBus } from '../../src/approval-gate/state-bus.js';
import { type IncomingCall, STATE_SCOPE, pendingKey } from '../../src/approval-gate/types.js';

function callRequiringApproval(): IncomingCall {
  return {
    session_id: 's1',
    function_call_id: 'tc-1',
    function_id: 'shell::exec',
    args: { command: 'date' },
    approval_required: ['shell::exec'],
    event_id: 'evt',
    reply_stream: 'replies',
  };
}

function callNotRequiringApproval(): IncomingCall {
  return {
    session_id: 's1',
    function_call_id: 'tc-1',
    function_id: 'shell::fs::ls',
    args: {},
    approval_required: ['shell::exec'],
    event_id: 'evt',
    reply_stream: 'replies',
  };
}

class ThrowingStateBus implements StateBus {
  async get(): Promise<unknown> {
    return null;
  }
  async set(): Promise<void> {
    throw new Error('boom');
  }
  async listPrefix(): Promise<unknown[]> {
    return [];
  }
}

describe('handleIntercept', () => {
  it('returns an approval-gate-marked allow reply when the call does not require approval', async () => {
    const bus = new InMemoryStateBus();
    const reply = (await handleIntercept(
      bus,
      STATE_SCOPE,
      callNotRequiringApproval(),
      1_000,
      60_000,
    )) as Record<string, unknown>;

    expect(reply.block).toBe(false);
    expect(reply.subscriber).toBe('approval-gate');
    expect(reply.approval_gate).toBe(true);
    expect(await bus.get(STATE_SCOPE, pendingKey('s1', 'tc-1'))).toBeNull();
  });

  it('writes a pending record and returns a marked pending block when approval is required', async () => {
    const bus = new InMemoryStateBus();
    const reply = (await handleIntercept(
      bus,
      STATE_SCOPE,
      callRequiringApproval(),
      1_000,
      60_000,
    )) as Record<string, unknown>;

    expect(reply.block).toBe(true);
    expect(reply.status).toBe('pending');
    expect(reply.subscriber).toBe('approval-gate');
    expect(reply.approval_gate).toBe(true);
    expect(reply.function_call_id).toBe('tc-1');
    expect(reply.function_id).toBe('shell::exec');

    const stored = (await bus.get(STATE_SCOPE, pendingKey('s1', 'tc-1'))) as Record<
      string,
      unknown
    >;
    expect(stored.status).toBe('pending');
    expect(stored.session_id).toBe('s1');
    expect(stored.created_at).toBe(1_000);
    expect(stored.expires_at).toBe(61_000);
  });

  it('returns a fail-closed denial envelope when the state bus write throws', async () => {
    const bus = new ThrowingStateBus();
    const reply = (await handleIntercept(
      bus,
      STATE_SCOPE,
      callRequiringApproval(),
      1_000,
      60_000,
    )) as Record<string, unknown>;

    expect(reply.block).toBe(true);
    expect(reply.status).toBe('denied');
    expect(reply.subscriber).toBe('approval-gate');
    expect(reply.approval_gate).toBe(true);
    const denial = reply.denial as Record<string, unknown>;
    expect(denial.kind).toBe('state_error');
    const detail = denial.detail as Record<string, unknown>;
    expect(detail.phase).toBe('pending_write');
    expect(detail.error).toBe('boom');
  });
});
