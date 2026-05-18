import { describe, expect, it, vi } from 'vitest';
import { handleResolve, handleResolveWithEvents } from '../../src/approval-gate/pending.js';
import type { ISdk } from '../../src/runtime/iii.js';

function fakeIii(): {
  iii: ISdk;
  setCalls: Array<{ scope: string; key: string; value: unknown }>;
  streamSets: unknown[];
} {
  const setCalls: Array<{ scope: string; key: string; value: unknown }> = [];
  const streamSets: unknown[] = [];
  const iii = {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      if (function_id === 'state::set') {
        setCalls.push(payload as { scope: string; key: string; value: unknown });
        return null;
      }
      if (function_id === 'stream::set') {
        streamSets.push(payload);
        return null;
      }
      return null;
    }),
  } as unknown as ISdk;
  return { iii, setCalls, streamSets };
}

describe('handleResolve (simplified)', () => {
  it('writes { decision, reason } to state via state::set on allow', async () => {
    const { iii, setCalls } = fakeIii();
    const out = await handleResolve(iii, 'approvals', {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'allow',
    });
    expect(out).toEqual({ ok: true });
    expect(setCalls).toHaveLength(1);
    expect(setCalls[0]).toEqual({
      scope: 'approvals',
      key: 's1/fc-1',
      value: { decision: 'allow', reason: null },
    });
  });

  it('preserves a reason when provided on deny', async () => {
    const { iii, setCalls } = fakeIii();
    const out = await handleResolve(iii, 'approvals', {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'deny',
      reason: 'user cancelled',
    });
    expect(out).toEqual({ ok: true });
    expect(setCalls[0]).toEqual({
      scope: 'approvals',
      key: 's1/fc-1',
      value: { decision: 'deny', reason: 'user cancelled' },
    });
  });

  it('returns missing_id when ids are absent', async () => {
    const { iii } = fakeIii();
    const out = await handleResolve(iii, 'approvals', { decision: 'allow' });
    expect(out).toEqual({ ok: false, error: 'missing_id' });
  });

  it('returns bad_decision when decision is invalid', async () => {
    const { iii } = fakeIii();
    const out = await handleResolve(iii, 'approvals', {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'maybe',
    });
    expect(out).toEqual({ ok: false, error: 'bad_decision' });
  });

  it('returns state_write_failed when state::set throws', async () => {
    const { iii } = fakeIii();
    (iii.trigger as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('boom'));
    const out = await handleResolve(iii, 'approvals', {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'allow',
    });
    expect(out).toEqual({ ok: false, error: 'state_write_failed' });
  });
});

describe('handleResolveWithEvents', () => {
  it('emits approval_resolved on success but does not publish turn::step_requested', async () => {
    const triggers: Array<{ function_id: string; payload: unknown }> = [];
    const iii = {
      trigger: vi.fn(
        async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
          triggers.push({ function_id, payload });
          return null;
        },
      ),
    } as unknown as ISdk;

    await handleResolveWithEvents(iii, 'approvals', {
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
    const iii = {
      trigger: vi.fn().mockRejectedValue(new Error('boom')),
    } as unknown as ISdk;
    const out = await handleResolveWithEvents(iii, 'approvals', {
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
