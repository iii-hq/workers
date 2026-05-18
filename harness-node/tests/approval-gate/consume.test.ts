import { describe, expect, it } from 'vitest';
import { handleConsume } from '../../src/approval-gate/consume.js';
import { InMemoryStateBus, type StateBus } from '../../src/approval-gate/state-bus.js';
import { STATE_SCOPE, buildPendingRecord, pendingKey } from '../../src/approval-gate/types.js';

async function seedResolved(
  bus: StateBus,
  session_id: string,
  function_call_id: string,
  decision: 'allow' | 'deny',
  reason: string | null = null,
): Promise<void> {
  const rec = buildPendingRecord(
    session_id,
    function_call_id,
    'shell::exec',
    { command: 'date' },
    0,
    60_000,
  );
  rec.status = 'resolved';
  rec.decision = decision;
  if (reason !== null) rec.reason = reason;
  rec.resolved_at = 1_000;
  await bus.set(STATE_SCOPE, pendingKey(session_id, function_call_id), rec);
}

class ThrowingListBus implements StateBus {
  async get(): Promise<unknown> {
    return null;
  }
  async set(): Promise<void> {}
  async listPrefix(): Promise<unknown[]> {
    throw new Error('list_boom');
  }
}

describe('handleConsume', () => {
  it('returns ok:false with missing_session_id when session_id is absent', async () => {
    const bus = new InMemoryStateBus();
    const out = (await handleConsume(bus, STATE_SCOPE, {})) as Record<string, unknown>;
    expect(out.ok).toBe(false);
    expect(out.error).toBe('missing_session_id');
    expect(out.entries).toEqual([]);
  });

  it('returns resolved entries and marks them consumed (second call returns empty)', async () => {
    const bus = new InMemoryStateBus();
    await seedResolved(bus, 's1', 'tc-1', 'allow');

    const first = (await handleConsume(bus, STATE_SCOPE, { session_id: 's1' })) as {
      ok: boolean;
      entries: Array<Record<string, unknown>>;
    };
    expect(first.ok).toBe(true);
    expect(first.entries).toHaveLength(1);
    expect(first.entries[0]?.function_call_id).toBe('tc-1');
    expect(first.entries[0]?.tool_call_id).toBe('tc-1');
    expect(first.entries[0]?.decision).toBe('allow');
    expect(first.entries[0]?.function_id).toBe('shell::exec');

    const second = (await handleConsume(bus, STATE_SCOPE, { session_id: 's1' })) as {
      ok: boolean;
      entries: unknown[];
    };
    expect(second.entries).toHaveLength(0);

    const stored = (await bus.get(STATE_SCOPE, pendingKey('s1', 'tc-1'))) as Record<
      string,
      unknown
    >;
    expect(stored.status).toBe('consumed');
    expect(typeof stored.consumed_at).toBe('number');
  });

  it('filters out non-resolved entries (pending, consumed)', async () => {
    const bus = new InMemoryStateBus();
    await bus.set(
      STATE_SCOPE,
      pendingKey('s1', 'tc-pending'),
      buildPendingRecord('s1', 'tc-pending', 'shell::exec', {}, 0, 60_000),
    );
    const consumed = buildPendingRecord('s1', 'tc-consumed', 'shell::exec', {}, 0, 60_000);
    consumed.status = 'consumed';
    await bus.set(STATE_SCOPE, pendingKey('s1', 'tc-consumed'), consumed);
    await seedResolved(bus, 's1', 'tc-resolved', 'deny', 'user');

    const out = (await handleConsume(bus, STATE_SCOPE, { session_id: 's1' })) as {
      entries: Array<Record<string, unknown>>;
    };
    expect(out.entries).toHaveLength(1);
    expect(out.entries[0]?.function_call_id).toBe('tc-resolved');
    expect(out.entries[0]?.reason).toBe('user');
  });

  it('filters out entries for other sessions', async () => {
    const bus = new InMemoryStateBus();
    await seedResolved(bus, 's1', 'tc-1', 'allow');
    await seedResolved(bus, 's2', 'tc-2', 'allow');

    const out = (await handleConsume(bus, STATE_SCOPE, { session_id: 's1' })) as {
      entries: Array<Record<string, unknown>>;
    };
    expect(out.entries).toHaveLength(1);
    expect(out.entries[0]?.function_call_id).toBe('tc-1');
  });

  it('returns ok:false with list_failed when bus.listPrefix throws (fail-closed)', async () => {
    const bus = new ThrowingListBus();
    const out = (await handleConsume(bus, STATE_SCOPE, { session_id: 's1' })) as Record<
      string,
      unknown
    >;
    expect(out.ok).toBe(false);
    expect(out.error).toBe('list_failed');
    expect(out.entries).toEqual([]);
  });
});
