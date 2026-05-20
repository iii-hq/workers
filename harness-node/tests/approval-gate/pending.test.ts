import { describe, expect, it, vi } from 'vitest';
import { handleResolve } from '../../src/approval-gate/pending.js';
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

  it('never emits to agent::events stream (denial flows through function_execution_end)', async () => {
    const { iii, streamSets } = fakeIii();
    await handleResolve(iii, 'approvals', {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'deny',
      reason: 'user cancelled',
    });
    expect(streamSets).toHaveLength(0);
  });
});
