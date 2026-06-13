/**
 * End-to-end pre_dispatch hook flow against the REAL chain (no
 * dispatchWithHook mocks): a fake `approval::gate` bound to
 * `harness::hook::pre_dispatch` holds/denies/allows calls, and
 * `harness::function::resolve` settles held ones — exactly the wire the
 * standalone approval-gate worker drives.
 */

import { describe, expect, it } from 'vitest';
import {
  createParallelApprovalHarness,
  executionEvents,
  makeAssistantWithCalls,
} from './parallel-approval-harness.js';

describe('pre_dispatch hook gate flow e2e', () => {
  it('hold parks the call with the gate-supplied HookInput, resolve-execute releases it without re-consulting', async () => {
    const h = createParallelApprovalHarness();
    // Default gate output is hold (1_800_000ms).

    h.seedExecute('sess-hold', makeAssistantWithCalls([{ id: 'fc-1', functionId: 'shell::run' }]));
    await h.runExecute('sess-hold');

    // The gate saw one pre_dispatch HookInput with the routing envelope.
    expect(h.gateCalls).toHaveLength(1);
    const rec = h.loadTurnRecord('sess-hold');
    expect(h.gateCalls[0]).toMatchObject({
      point: 'pre_dispatch',
      session_id: 'sess-hold',
      turn_id: rec?.turn_id,
      depth: 0,
      call: {
        id: 'fc-1',
        function_id: 'shell::run',
        // Routing envelope: caller args + session/call identity at top level.
        arguments: {
          session_id: 'sess-hold',
          function_call_id: 'fc-1',
          function_id: 'shell::run',
        },
      },
    });
    expect(rec?.state).toBe('function_awaiting_approval');
    expect(rec?.awaiting_approval?.map((e) => e.function_call_id)).toEqual(['fc-1']);

    const out = await h.resolveApproval('sess-hold', 'fc-1', 'allow');
    expect(out).toEqual({ resolved: true, turn_resumed: true });

    const done = h.loadTurnRecord('sess-hold');
    expect(done?.state).toBe('assistant_streaming');
    expect(executionEvents(h.emitted, 'function_execution_end', 'fc-1')).toHaveLength(1);
    // Released calls do NOT re-consult the hook chain.
    expect(h.gateCalls).toHaveLength(1);
  });

  it('gate deny answers the call inline with a denied result, nothing parks', async () => {
    const h = createParallelApprovalHarness();
    h.setGateOutput({ decision: 'deny', reason: 'blocked by policy: no shell' });

    h.seedExecute('sess-deny', makeAssistantWithCalls([{ id: 'fc-1', functionId: 'shell::run' }]));
    await h.runExecute('sess-deny');

    const rec = h.loadTurnRecord('sess-deny');
    expect(rec?.state).toBe('assistant_streaming');
    expect(rec?.awaiting_approval ?? []).toEqual([]);
    const sessionMessages = h.sessions
      .messageEntries('sess-deny')
      .map((e) => e.message as { role?: string; is_error?: boolean; details?: unknown });
    const denial = sessionMessages.find((m) => m.role === 'function_result');
    expect(denial?.is_error).toBe(true);
    expect(denial?.details).toMatchObject({
      status: 'denied',
      denied_by: 'hook',
      reason: 'blocked by policy: no shell',
    });
  });

  it('gate continue lets the call dispatch straight through', async () => {
    const h = createParallelApprovalHarness();
    h.setGateOutput({ decision: 'continue' });

    h.seedExecute('sess-allow', makeAssistantWithCalls([{ id: 'fc-1', functionId: 'shell::run' }]));
    await h.runExecute('sess-allow');

    const rec = h.loadTurnRecord('sess-allow');
    expect(rec?.state).toBe('assistant_streaming');
    expect(executionEvents(h.emitted, 'function_execution_end', 'fc-1')).toHaveLength(1);
  });

  it('a crashed gate fails closed (deny, not allow, not hold)', async () => {
    const h = createParallelApprovalHarness();
    h.setGateOutput(() => {
      throw new Error('gate worker died');
    });

    h.seedExecute('sess-down', makeAssistantWithCalls([{ id: 'fc-1', functionId: 'shell::run' }]));
    await h.runExecute('sess-down');

    const rec = h.loadTurnRecord('sess-down');
    expect(rec?.state).toBe('assistant_streaming');
    expect(rec?.work).toBeUndefined();
    const denial = h.sessions
      .messageEntries('sess-down')
      .map((e) => e.message as { role?: string; details?: Record<string, unknown> })
      .find((m) => m.role === 'function_result');
    expect(denial?.details).toMatchObject({ denied_by: 'gate_unavailable' });
  });

  it('resolve-deny delivers the DenialEnvelope as an is_error function result', async () => {
    const h = createParallelApprovalHarness();

    h.seedExecute(
      'sess-userdeny',
      makeAssistantWithCalls([{ id: 'fc-1', functionId: 'shell::run' }]),
    );
    await h.runExecute('sess-userdeny');
    await h.resolveApproval('sess-userdeny', 'fc-1', 'deny', 'too risky');

    const rec = h.loadTurnRecord('sess-userdeny');
    expect(rec?.state).toBe('assistant_streaming');
    const denial = h.sessions
      .messageEntries('sess-userdeny')
      .map((e) => e.message as { role?: string; is_error?: boolean; details?: unknown })
      .find((m) => m.role === 'function_result');
    expect(denial?.is_error).toBe(true);
    expect(denial?.details).toMatchObject({
      status: 'denied',
      denied_by: 'user',
      function_id: 'shell::run',
      reason: 'too risky',
    });
  });
});
