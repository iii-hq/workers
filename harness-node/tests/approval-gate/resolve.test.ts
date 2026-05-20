import { describe, expect, it, vi } from 'vitest';
import { handleResolveRequest } from '../../src/approval-gate/resolve.js';
import { fakeIii } from './_helpers/fakeIii.js';

describe('handleResolveRequest', () => {
  it('triggers turn::approval_resume::s1/fc-1 with decision payload on allow', async () => {
    const { iii, resumeCalls } = fakeIii();
    const out = await handleResolveRequest(iii, {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'allow',
    });
    expect(out).toEqual({ ok: true });
    expect(resumeCalls).toHaveLength(1);
    expect(resumeCalls[0]).toEqual({
      function_id: 'turn::approval_resume::s1/fc-1',
      payload: { decision: 'allow', reason: null },
    });
  });

  it('preserves a reason when provided on deny', async () => {
    const { iii, resumeCalls } = fakeIii();
    const out = await handleResolveRequest(iii, {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'deny',
      reason: 'user cancelled',
    });
    expect(out).toEqual({ ok: true });
    expect(resumeCalls[0]).toEqual({
      function_id: 'turn::approval_resume::s1/fc-1',
      payload: { decision: 'deny', reason: 'user cancelled' },
    });
  });

  it('accepts tool_call_id as a fallback (legacy wire alias)', async () => {
    const { iii, resumeCalls } = fakeIii();
    const out = await handleResolveRequest(iii, {
      session_id: 's1',
      tool_call_id: 'legacy-fc-1',
      decision: 'allow',
    });
    expect(out).toEqual({ ok: true });
    expect(resumeCalls[0]?.function_id).toBe('turn::approval_resume::s1/legacy-fc-1');
  });

  it("returns 'resume_failed' when resume fn trigger throws", async () => {
    const { iii } = fakeIii();
    (iii.trigger as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('boom'));
    const out = await handleResolveRequest(iii, {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'allow',
    });
    expect(out).toEqual({ ok: false, error: 'resume_failed' });
  });

  it('never emits to the agent::events stream (denial flows through function_execution_end)', async () => {
    const { iii, streamSets } = fakeIii();
    await handleResolveRequest(iii, {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'deny',
      reason: 'user cancelled',
    });
    expect(streamSets).toHaveLength(0);
  });
});
