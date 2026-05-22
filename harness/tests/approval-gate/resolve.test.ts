/**
 * Adversarial tests for the approval::resolve handler. The handler is a
 * trust boundary: a hostile or malformed resolve payload must never crash
 * the worker, never silently fire a resume, and never resolve to the wrong
 * per-call resume function. Its only sanctioned outputs are a clean
 * `{ ok: true }` or a typed `{ ok: false, error }`.
 */
import { describe, expect, it, vi } from 'vitest';
import { handleResolveRequest } from '../../src/approval-gate/resolve.js';
import { fakeIii } from './_helpers/fakeIii.js';

describe('handleResolveRequest — routing the decision', () => {
  it('routes to the exact per-call resume fn with a normalized payload', async () => {
    const { iii, resumeCalls } = fakeIii();
    const out = await handleResolveRequest(iii, {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'deny',
      reason: 'user cancelled',
    });
    expect(out).toEqual({ ok: true });
    expect(resumeCalls).toEqual([
      {
        function_id: 'turn::approval_resume::s1/fc-1',
        payload: { decision: 'deny', reason: 'user cancelled' },
      },
    ]);
  });

  it('prefers function_call_id over a conflicting legacy tool_call_id', async () => {
    const { iii, resumeCalls } = fakeIii();
    await handleResolveRequest(iii, {
      session_id: 's1',
      function_call_id: 'canonical',
      tool_call_id: 'legacy',
      decision: 'allow',
    });
    expect(resumeCalls[0]?.function_id).toBe('turn::approval_resume::s1/canonical');
  });

  it('never emits to the agent::events stream (denial flows via execution_end)', async () => {
    const { iii, streamSets } = fakeIii();
    await handleResolveRequest(iii, {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'deny',
      reason: 'nope',
    });
    expect(streamSets).toHaveLength(0);
  });
});

describe('handleResolveRequest — hostile / malformed input is rejected, not crashed', () => {
  it('returns invalid_payload (does not throw) when session_id contains a slash', async () => {
    // "/" is the reserved separator in the state key. A slashed id must be
    // refused at the boundary — the handler promise must resolve to a typed
    // error, never reject out of the worker.
    const { iii, calls } = fakeIii();
    await expect(
      handleResolveRequest(iii, {
        session_id: 'a/b',
        function_call_id: 'fc',
        decision: 'allow',
      }),
    ).resolves.toEqual({ ok: false, error: 'invalid_payload' });
    expect(calls).toHaveLength(0);
  });

  it('returns invalid_payload when the resolved id (via tool_call_id) contains a slash', async () => {
    const { iii, calls } = fakeIii();
    const out = await handleResolveRequest(iii, {
      session_id: 's1',
      tool_call_id: 'fc/evil',
      decision: 'allow',
    });
    expect(out).toEqual({ ok: false, error: 'invalid_payload' });
    expect(calls).toHaveLength(0);
  });

  it('returns invalid_payload and fires nothing when both ids are missing', async () => {
    const { iii, calls } = fakeIii();
    const out = await handleResolveRequest(iii, {
      session_id: 's1',
      decision: 'allow',
    } as never);
    expect(out).toEqual({ ok: false, error: 'invalid_payload' });
    expect(calls).toHaveLength(0);
  });

  it('returns invalid_payload for an out-of-enum decision', async () => {
    const { iii, calls } = fakeIii();
    const out = await handleResolveRequest(iii, {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'maybe',
    } as never);
    expect(out).toEqual({ ok: false, error: 'invalid_payload' });
    expect(calls).toHaveLength(0);
  });
});

describe('handleResolveRequest — downstream failure is surfaced as resume_failed', () => {
  it('returns resume_failed when the resume trigger rejects', async () => {
    const { iii } = fakeIii();
    (iii.trigger as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('boom'));
    const out = await handleResolveRequest(iii, {
      session_id: 's1',
      function_call_id: 'fc-1',
      decision: 'allow',
    });
    expect(out).toEqual({ ok: false, error: 'resume_failed' });
  });
});
