import { describe, expect, it, vi } from 'vitest';
import { handleResolve } from '../../src/approval-gate/resolve.js';
import { fakeIii } from './_helpers/fakeIii.js';

describe('handleResolve', () => {
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

  it('unwraps engine { body } envelope', async () => {
    const { iii, setCalls } = fakeIii();
    const out = await handleResolve(iii, 'approvals', {
      body: { session_id: 's1', function_call_id: 'fc-1', decision: 'allow' },
    });
    expect(out).toEqual({ ok: true });
    expect(setCalls[0].key).toBe('s1/fc-1');
  });

  it('accepts tool_call_id as a fallback (legacy wire alias)', async () => {
    const { iii, setCalls } = fakeIii();
    const out = await handleResolve(iii, 'approvals', {
      session_id: 's1',
      tool_call_id: 'legacy-fc-1',
      decision: 'allow',
    });
    expect(out).toEqual({ ok: true });
    expect(setCalls[0].key).toBe('s1/legacy-fc-1');
  });

  // R1 — wire-contract regression guards. Every error code must match
  // today's string exactly so downstream telemetry / future consumers
  // don't break silently.
  describe('wire error strings (R1)', () => {
    it("returns 'missing_id' when payload is null", async () => {
      const { iii } = fakeIii();
      const out = await handleResolve(iii, 'approvals', null);
      expect(out).toEqual({ ok: false, error: 'missing_id' });
    });

    it("returns 'missing_id' when session_id is absent", async () => {
      const { iii } = fakeIii();
      const out = await handleResolve(iii, 'approvals', {
        function_call_id: 'fc-1',
        decision: 'allow',
      });
      expect(out).toEqual({ ok: false, error: 'missing_id' });
    });

    it("returns 'missing_id' when both function_call_id and tool_call_id absent", async () => {
      const { iii } = fakeIii();
      const out = await handleResolve(iii, 'approvals', {
        session_id: 's1',
        decision: 'allow',
      });
      expect(out).toEqual({ ok: false, error: 'missing_id' });
    });

    it("returns 'bad_decision' when decision is not allow/deny", async () => {
      const { iii } = fakeIii();
      const out = await handleResolve(iii, 'approvals', {
        session_id: 's1',
        function_call_id: 'fc-1',
        decision: 'maybe',
      });
      expect(out).toEqual({ ok: false, error: 'bad_decision' });
    });

    it("returns 'state_write_failed' when state::set throws", async () => {
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

  it('never emits to the agent::events stream (denial flows through function_execution_end)', async () => {
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
