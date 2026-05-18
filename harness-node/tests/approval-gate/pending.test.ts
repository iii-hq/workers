import { describe, expect, it, vi } from 'vitest';
import { handleResolve, handleResolveWithEvents } from '../../src/approval-gate/pending.js';
import { InMemoryStateBus } from '../../src/approval-gate/state-bus.js';
import type { ISdk } from '../../src/runtime/iii.js';

describe('handleResolve (simplified)', () => {
  it('writes { decision, reason } to the state-bus on allow', async () => {
    const bus = new InMemoryStateBus();
    const out = await handleResolve(bus, 'approvals', {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'allow',
    });
    expect(out).toEqual({ ok: true });
    expect(await bus.get('approvals', 's1/fc-1')).toEqual({ decision: 'allow', reason: null });
  });

  it('preserves a reason when provided on deny', async () => {
    const bus = new InMemoryStateBus();
    const out = await handleResolve(bus, 'approvals', {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'deny',
      reason: 'user cancelled',
    });
    expect(out).toEqual({ ok: true });
    expect(await bus.get('approvals', 's1/fc-1')).toEqual({
      decision: 'deny',
      reason: 'user cancelled',
    });
  });

  it('returns missing_id when ids are absent', async () => {
    const bus = new InMemoryStateBus();
    const out = await handleResolve(bus, 'approvals', { decision: 'allow' });
    expect(out).toEqual({ ok: false, error: 'missing_id' });
  });

  it('returns bad_decision when decision is invalid', async () => {
    const bus = new InMemoryStateBus();
    const out = await handleResolve(bus, 'approvals', {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'maybe',
    });
    expect(out).toEqual({ ok: false, error: 'bad_decision' });
  });

  it('returns state_write_failed when bus.set throws', async () => {
    const bus = new InMemoryStateBus();
    vi.spyOn(bus, 'set').mockRejectedValue(new Error('boom'));
    const out = await handleResolve(bus, 'approvals', {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'allow',
    });
    expect(out).toEqual({ ok: false, error: 'state_write_failed' });
  });
});

describe('handleResolveWithEvents', () => {
  it('emits approval_resolved on success but does not publish turn::step_requested', async () => {
    const bus = new InMemoryStateBus();
    const triggers: Array<{ function_id: string; payload: unknown }> = [];
    const iii = {
      trigger: vi.fn(
        async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
          triggers.push({ function_id, payload });
          return null;
        },
      ),
    } as unknown as ISdk;

    await handleResolveWithEvents(iii, bus, 'approvals', {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'allow',
      reason: 'looks good',
    });

    const fns = triggers.map((t) => t.function_id);
    expect(fns).toContain('stream::set');
    expect(fns).not.toContain('iii::durable::publish');

    const resolved = triggers
      .filter((t) => t.function_id === 'stream::set')
      .map((t) => (t.payload as Record<string, unknown>).data as Record<string, unknown>)
      .find((d) => d.type === 'approval_resolved');
    expect(resolved).toBeDefined();
    expect(resolved?.function_call_id).toBe('fc-1');
    expect(resolved?.tool_call_id).toBe('fc-1');
    expect(resolved?.decision).toBe('allow');
    expect(resolved?.reason).toBe('looks good');
  });

  it('does not publish when state write fails', async () => {
    const bus = new InMemoryStateBus();
    vi.spyOn(bus, 'set').mockRejectedValue(new Error('boom'));
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    const out = await handleResolveWithEvents(iii, bus, 'approvals', {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'allow',
    });
    expect((out as Record<string, unknown>).ok).toBe(false);
    const fns = (iii.trigger as ReturnType<typeof vi.fn>).mock.calls.map(
      (c) => (c[0] as { function_id: string }).function_id,
    );
    expect(fns).not.toContain('iii::durable::publish');
  });
});
