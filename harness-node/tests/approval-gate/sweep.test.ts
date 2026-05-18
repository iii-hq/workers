import { describe, expect, it } from 'vitest';
import { InMemoryStateBus, type StateBus } from '../../src/approval-gate/state-bus.js';
import { handleSweepSession } from '../../src/approval-gate/sweep.js';
import { STATE_SCOPE, buildPendingRecord, pendingKey } from '../../src/approval-gate/types.js';

class ThrowingListBus implements StateBus {
  async get(): Promise<unknown> {
    return null;
  }
  async set(): Promise<void> {}
  async listPrefix(): Promise<unknown[]> {
    throw new Error('list_boom');
  }
}

describe('handleSweepSession', () => {
  it('returns ok:false with missing_session_id when session_id is absent', async () => {
    const bus = new InMemoryStateBus();
    const out = (await handleSweepSession(bus, STATE_SCOPE, {})) as Record<string, unknown>;
    expect(out.ok).toBe(false);
    expect(out.error).toBe('missing_session_id');
    expect(out.swept).toBe(0);
  });

  it('sweeps pending entries for the session as resolved+deny+timed_out', async () => {
    const bus = new InMemoryStateBus();
    await bus.set(
      STATE_SCOPE,
      pendingKey('s1', 'tc-1'),
      buildPendingRecord('s1', 'tc-1', 'shell::exec', {}, 0, 60_000),
    );
    await bus.set(
      STATE_SCOPE,
      pendingKey('s1', 'tc-2'),
      buildPendingRecord('s1', 'tc-2', 'shell::exec', {}, 0, 60_000),
    );

    const out = (await handleSweepSession(bus, STATE_SCOPE, { session_id: 's1' })) as Record<
      string,
      unknown
    >;
    expect(out.ok).toBe(true);
    expect(out.swept).toBe(2);

    for (const fcid of ['tc-1', 'tc-2']) {
      const stored = (await bus.get(STATE_SCOPE, pendingKey('s1', fcid))) as Record<
        string,
        unknown
      >;
      expect(stored.status).toBe('resolved');
      expect(stored.decision).toBe('deny');
      expect(stored.reason).toBe('timed_out');
      expect(typeof stored.resolved_at).toBe('number');
    }
  });

  it('leaves entries from other sessions untouched', async () => {
    const bus = new InMemoryStateBus();
    await bus.set(
      STATE_SCOPE,
      pendingKey('s1', 'tc-1'),
      buildPendingRecord('s1', 'tc-1', 'shell::exec', {}, 0, 60_000),
    );
    await bus.set(
      STATE_SCOPE,
      pendingKey('s2', 'tc-2'),
      buildPendingRecord('s2', 'tc-2', 'shell::exec', {}, 0, 60_000),
    );

    const out = (await handleSweepSession(bus, STATE_SCOPE, { session_id: 's1' })) as Record<
      string,
      unknown
    >;
    expect(out.swept).toBe(1);

    const otherSession = (await bus.get(STATE_SCOPE, pendingKey('s2', 'tc-2'))) as Record<
      string,
      unknown
    >;
    expect(otherSession.status).toBe('pending');
  });

  it('leaves non-pending entries untouched', async () => {
    const bus = new InMemoryStateBus();
    const resolved = buildPendingRecord('s1', 'tc-1', 'shell::exec', {}, 0, 60_000);
    resolved.status = 'resolved';
    resolved.decision = 'allow';
    await bus.set(STATE_SCOPE, pendingKey('s1', 'tc-1'), resolved);

    const out = (await handleSweepSession(bus, STATE_SCOPE, { session_id: 's1' })) as Record<
      string,
      unknown
    >;
    expect(out.swept).toBe(0);

    const stored = (await bus.get(STATE_SCOPE, pendingKey('s1', 'tc-1'))) as Record<
      string,
      unknown
    >;
    expect(stored.decision).toBe('allow');
  });

  it('returns ok:false with list_failed when bus.listPrefix throws (fail-closed)', async () => {
    const bus = new ThrowingListBus();
    const out = (await handleSweepSession(bus, STATE_SCOPE, { session_id: 's1' })) as Record<
      string,
      unknown
    >;
    expect(out.ok).toBe(false);
    expect(out.error).toBe('list_failed');
    expect(out.swept).toBe(0);
  });
});
