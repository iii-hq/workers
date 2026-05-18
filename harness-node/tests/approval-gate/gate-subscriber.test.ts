import { describe, expect, it } from 'vitest';
import type { ApprovalGateConfig } from '../../src/approval-gate/config.js';
import { handleGateEvent } from '../../src/approval-gate/gate-subscriber.js';
import { InMemoryStateBus } from '../../src/approval-gate/state-bus.js';
import { STATE_SCOPE, pendingKey } from '../../src/approval-gate/types.js';
import type { ISdk } from '../../src/runtime/iii.js';

type TriggerCall = { function_id: string; payload: unknown };

function fakeIii(handler: (call: TriggerCall) => unknown = () => undefined): {
  iii: ISdk;
  calls: TriggerCall[];
} {
  const calls: TriggerCall[] = [];
  const iii = {
    trigger: async <T, R>(req: { function_id: string; payload: T }): Promise<R> => {
      const call = { function_id: req.function_id, payload: req.payload };
      calls.push(call);
      const result = handler(call);
      if (result instanceof Error) throw result;
      return result as R;
    },
  } as unknown as ISdk;
  return { iii, calls };
}

const cfg: ApprovalGateConfig = {
  approval_state_scope: STATE_SCOPE,
  default_timeout_ms: 60_000,
  policy_function_id: 'policy::check_permissions',
  topic: 'agent::before_function_call',
};

function approvalRequiredEnvelope() {
  return {
    event_id: 'evt-1',
    reply_stream: 'rs-1',
    payload: {
      session_id: 's1',
      function_call: { id: 'tc-1', function_id: 'shell::exec', arguments: { command: 'date' } },
      approval_required: ['shell::exec'],
    },
  };
}

describe('handleGateEvent (post PR #150)', () => {
  it('returns {block:false} for an envelope it cannot parse', async () => {
    const { iii } = fakeIii();
    const reply = (await handleGateEvent(
      { iii, bus: new InMemoryStateBus(), cfg },
      { not: 'an envelope' },
    )) as Record<string, unknown>;
    expect(reply.block).toBe(false);
  });

  it('does not write to state-bus on needs_approval; only emits event and returns pending', async () => {
    const bus = new InMemoryStateBus();
    const { iii, calls } = fakeIii((call) => {
      if (call.function_id === 'policy::check_permissions') return null; // legacy fallback
      return undefined;
    });
    const reply = (await handleGateEvent({ iii, bus, cfg }, approvalRequiredEnvelope())) as Record<
      string,
      unknown
    >;

    expect(reply.block).toBe(true);
    expect(reply.status).toBe('pending');
    expect(reply.subscriber).toBe('approval-gate');
    expect(reply.approval_gate).toBe(true);

    expect(await bus.get(STATE_SCOPE, pendingKey('s1', 'tc-1'))).toBeNull();

    const requestedEvent = calls
      .filter((c) => c.function_id === 'stream::set')
      .map((c) => (c.payload as Record<string, unknown>).data as Record<string, unknown>)
      .find((d) => d.type === 'approval_requested');
    expect(requestedEvent).toBeDefined();
    expect(requestedEvent?.function_call_id).toBe('tc-1');
    expect(requestedEvent).not.toHaveProperty('expires_at');
  });

  it('stays stateless when policy needs approval even if approval_required is empty', async () => {
    const bus = new InMemoryStateBus();
    const { iii } = fakeIii((call) => {
      if (call.function_id === 'policy::check_permissions') {
        return { decision: 'needs_approval' };
      }
      return undefined;
    });
    const envelope = {
      event_id: 'evt-1',
      reply_stream: 'rs-1',
      payload: {
        session_id: 's1',
        function_call: { id: 'tc-1', function_id: 'shell::exec', arguments: { command: 'date' } },
        approval_required: [],
      },
    };
    const reply = (await handleGateEvent({ iii, bus, cfg }, envelope)) as Record<string, unknown>;

    expect(reply.block).toBe(true);
    expect(reply.status).toBe('pending');
    expect(reply.approval_gate).toBe(true);
    expect(await bus.get(STATE_SCOPE, pendingKey('s1', 'tc-1'))).toBeNull();
  });

  it('returns a marked allow reply when policy explicitly returns allow', async () => {
    const bus = new InMemoryStateBus();
    const { iii } = fakeIii((call) => {
      if (call.function_id === 'policy::check_permissions') {
        return { decision: 'allow', rule_id: 'test/allow' };
      }
      return undefined;
    });
    const envelope = {
      event_id: 'evt-1',
      reply_stream: 'rs-1',
      payload: {
        session_id: 's1',
        function_call: { id: 'tc-1', function_id: 'shell::fs::ls', arguments: {} },
        approval_required: ['shell::exec'],
      },
    };
    const reply = (await handleGateEvent({ iii, bus, cfg }, envelope)) as Record<string, unknown>;

    expect(reply.block).toBe(false);
    expect(reply.subscriber).toBe('approval-gate');
    expect(reply.approval_gate).toBe(true);
    expect(await bus.get(STATE_SCOPE, pendingKey('s1', 'tc-1'))).toBeNull();
  });

  it('returns pending on needs_approval even when approval_required is empty (policy is source of truth)', async () => {
    const bus = new InMemoryStateBus();
    const { iii } = fakeIii((call) => {
      if (call.function_id === 'policy::check_permissions') {
        return { decision: 'needs_approval' };
      }
      return undefined;
    });
    const envelope = {
      event_id: 'evt-1',
      reply_stream: 'rs-1',
      payload: {
        session_id: 's1',
        function_call: { id: 'tc-1', function_id: 'shell::exec', arguments: { command: 'date' } },
        approval_required: [],
      },
    };
    const reply = (await handleGateEvent({ iii, bus, cfg }, envelope)) as Record<string, unknown>;

    expect(reply.block).toBe(true);
    expect(reply.status).toBe('pending');
    expect(await bus.get(STATE_SCOPE, pendingKey('s1', 'tc-1'))).toBeNull();
  });

  it('returns a deny envelope when policy denies', async () => {
    const bus = new InMemoryStateBus();
    const { iii } = fakeIii((call) => {
      if (call.function_id === 'policy::check_permissions') {
        return {
          decision: 'deny',
          rule_id: 'deny-rule',
          matched_constraint: { field: 'command', operator: 'eq', value: 'rm -rf /' },
        };
      }
      return undefined;
    });
    const reply = (await handleGateEvent({ iii, bus, cfg }, approvalRequiredEnvelope())) as Record<
      string,
      unknown
    >;

    expect(reply.block).toBe(true);
    expect(reply.subscriber).toBe('approval-gate');
    expect(reply.approval_gate).toBe(true);
    expect(reply.denial).toBeDefined();
  });
});
