import { describe, expect, it } from 'vitest';
import {
  awaitDecision,
  handleListPending,
  handleResolve,
} from '../../src/approval-gate/pending.js';
import { InMemoryStateBus } from '../../src/approval-gate/state-bus.js';
import { buildPendingRecord, pendingKey } from '../../src/approval-gate/types.js';

describe('handleResolve', () => {
  it('flips status from pending to allow', async () => {
    const bus = new InMemoryStateBus();
    await bus.set(
      'approvals',
      pendingKey('s1', 'tc-1'),
      buildPendingRecord('tc-1', 'write', {}, 0, 60_000),
    );
    const out = (await handleResolve(bus, 'approvals', {
      function_call_id: 'tc-1',
      session_id: 's1',
      decision: 'allow',
    })) as Record<string, unknown>;
    expect(out.ok).toBe(true);
    const stored = (await bus.get('approvals', pendingKey('s1', 'tc-1'))) as Record<
      string,
      unknown
    >;
    expect(stored.status).toBe('allow');
  });

  it('rejects already-resolved entries', async () => {
    const bus = new InMemoryStateBus();
    const rec = buildPendingRecord('tc-1', 'write', {}, 0, 60_000);
    rec.status = 'allow';
    await bus.set('approvals', pendingKey('s1', 'tc-1'), rec);
    const out = (await handleResolve(bus, 'approvals', {
      session_id: 's1',
      function_call_id: 'tc-1',
      decision: 'deny',
    })) as Record<string, unknown>;
    expect(out.ok).toBe(false);
    expect(out.error).toBe('already_resolved');
  });

  it('errors on missing id or bad decision', async () => {
    const bus = new InMemoryStateBus();
    expect(((await handleResolve(bus, 'approvals', {})) as Record<string, unknown>).error).toBe(
      'missing_id',
    );
    expect(
      (
        (await handleResolve(bus, 'approvals', {
          session_id: 's1',
          function_call_id: 'tc-1',
          decision: 'weird',
        })) as Record<string, unknown>
      ).error,
    ).toBe('bad_decision');
  });
});

describe('handleListPending', () => {
  it('returns only pending entries for the session', async () => {
    const bus = new InMemoryStateBus();
    await bus.set(
      'approvals',
      pendingKey('s1', 'tc-1'),
      buildPendingRecord('tc-1', 'write', {}, 0, 60_000),
    );
    const resolved = buildPendingRecord('tc-2', 'write', {}, 0, 60_000);
    resolved.status = 'allow';
    await bus.set('approvals', pendingKey('s1', 'tc-2'), resolved);
    await bus.set(
      'approvals',
      pendingKey('other', 'tc-3'),
      buildPendingRecord('tc-3', 'write', {}, 0, 60_000),
    );
    const out = (await handleListPending(bus, 'approvals', { session_id: 's1' })) as {
      pending: Array<Record<string, unknown>>;
    };
    expect(out.pending).toHaveLength(1);
    expect(out.pending[0]?.function_call_id).toBe('tc-1');
  });
});

describe('awaitDecision', () => {
  it('returns Allow when status flips', async () => {
    const bus = new InMemoryStateBus();
    const key = pendingKey('s1', 'tc-1');
    await bus.set('approvals', key, buildPendingRecord('tc-1', 'write', {}, Date.now(), 5_000));
    setTimeout(async () => {
      const rec = (await bus.get('approvals', key)) as Record<string, unknown>;
      rec.status = 'allow';
      await bus.set('approvals', key, rec);
    }, 30);
    const out = await awaitDecision(bus, 'approvals', 's1', 'tc-1', Date.now() + 5_000);
    expect(out.kind).toBe('allow');
  });

  it('times out when expires_at passes with status still pending', async () => {
    const bus = new InMemoryStateBus();
    await bus.set(
      'approvals',
      pendingKey('s1', 'tc-1'),
      buildPendingRecord('tc-1', 'write', {}, 0, 0),
    );
    const out = await awaitDecision(bus, 'approvals', 's1', 'tc-1', Date.now() - 10);
    expect(out.kind).toBe('deny');
    if (out.kind === 'deny') expect(out.reason).toBe('timeout');
  });

  it('fails closed on missing record', async () => {
    const bus = new InMemoryStateBus();
    const out = await awaitDecision(bus, 'approvals', 's1', 'tc-1', Date.now() + 1_000);
    expect(out.kind).toBe('deny');
    if (out.kind === 'deny') expect(out.reason).toBe('state_unavailable');
  });
});
