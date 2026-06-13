import { afterEach, describe, expect, it, vi } from 'vitest';
import * as agentTriggerModule from '../../src/turn-orchestrator/agent-trigger.js';
import {
  createParallelApprovalHarness,
  executionEvents,
  makeAssistantWithCalls,
} from './parallel-approval-harness.js';

afterEach(() => {
  vi.restoreAllMocks();
});

/** Count `turn::function_awaiting_approval` re-wakes enqueued for a session. */
function wakeEnqueues(
  h: ReturnType<typeof createParallelApprovalHarness>,
  sessionId: string,
): number {
  const trigger = h.iii.trigger as unknown as {
    mock: {
      calls: Array<[{ function_id?: string; payload?: { session_id?: string }; action?: unknown }]>;
    };
  };
  return trigger.mock.calls.filter(
    ([arg]) =>
      arg?.function_id === 'turn::function_awaiting_approval' &&
      arg?.payload?.session_id === sessionId &&
      arg?.action != null,
  ).length;
}

describe('parallel approval e2e', () => {
  it('dispatches later calls while earlier ones park without blocking the batch', async () => {
    const h = createParallelApprovalHarness();
    vi.spyOn(agentTriggerModule, 'dispatchWithHook')
      .mockResolvedValueOnce({
        kind: 'pending',
        held_by: 'approval::gate',
        pending_timeout_ms: 1_800_000,
      })
      .mockResolvedValueOnce({
        kind: 'result',
        result: {
          content: [{ type: 'text' as const, text: 'fc-2-ok' }],
          details: {},
          terminate: false,
        },
      })
      .mockResolvedValueOnce({
        kind: 'pending',
        held_by: 'approval::gate',
        pending_timeout_ms: 1_800_000,
      });

    h.seedExecute(
      'sess-parallel',
      makeAssistantWithCalls([
        { id: 'fc-1', functionId: 'shell::run' },
        { id: 'fc-2', functionId: 'shell::run' },
        { id: 'fc-3', functionId: 'shell::run' },
      ]),
    );

    await h.runExecute('sess-parallel');
    const rec = h.loadTurnRecord('sess-parallel');

    expect(rec?.state).toBe('function_awaiting_approval');
    expect(rec?.awaiting_approval?.map((e) => e.function_call_id).sort()).toEqual(['fc-1', 'fc-3']);
    expect(rec?.work?.executed['fc-2']?.result.content[0]).toMatchObject({ text: 'fc-2-ok' });
    expect(rec?.work?.executed['fc-1']).toBeUndefined();
    expect(rec?.work?.executed['fc-3']).toBeUndefined();
  });

  it('executes one approved call immediately while a sibling stays pending', async () => {
    const h = createParallelApprovalHarness();
    vi.spyOn(agentTriggerModule, 'dispatchWithHook')
      .mockResolvedValueOnce({
        kind: 'pending',
        held_by: 'approval::gate',
        pending_timeout_ms: 1_800_000,
      })
      .mockResolvedValueOnce({
        kind: 'result',
        result: { content: [{ type: 'text' as const, text: 'ok' }], details: {}, terminate: false },
      })
      .mockResolvedValueOnce({
        kind: 'pending',
        held_by: 'approval::gate',
        pending_timeout_ms: 1_800_000,
      });

    h.seedExecute(
      'sess-partial',
      makeAssistantWithCalls([
        { id: 'fc-1', functionId: 'shell::run' },
        { id: 'fc-2', functionId: 'shell::run' },
        { id: 'fc-3', functionId: 'shell::run' },
      ]),
    );
    await h.runExecute('sess-partial');

    const fc1StartsBefore = executionEvents(h.emitted, 'function_execution_start', 'fc-1');
    expect(fc1StartsBefore).toHaveLength(1);
    await h.resolveApproval('sess-partial', 'fc-1', 'allow');

    const rec = h.loadTurnRecord('sess-partial');
    expect(rec?.state).toBe('function_awaiting_approval');
    expect(rec?.awaiting_approval?.map((e) => e.function_call_id)).toEqual(['fc-3']);
    expect(rec?.work?.executed['fc-1']).toBeDefined();
    expect(rec?.work?.executed['fc-3']).toBeUndefined();

    expect(executionEvents(h.emitted, 'function_execution_start', 'fc-1')).toHaveLength(1);
    expect(executionEvents(h.emitted, 'function_execution_end', 'fc-1')).toHaveLength(1);
  });

  it('resolves approvals out of order without waiting for batch order', async () => {
    const h = createParallelApprovalHarness();
    vi.spyOn(agentTriggerModule, 'dispatchWithHook')
      .mockResolvedValueOnce({
        kind: 'pending',
        held_by: 'approval::gate',
        pending_timeout_ms: 1_800_000,
      })
      .mockResolvedValueOnce({
        kind: 'pending',
        held_by: 'approval::gate',
        pending_timeout_ms: 1_800_000,
      });

    h.seedExecute(
      'sess-order',
      makeAssistantWithCalls([
        { id: 'fc-1', functionId: 'shell::run' },
        { id: 'fc-2', functionId: 'shell::run' },
      ]),
    );
    await h.runExecute('sess-order');

    await h.resolveApproval('sess-order', 'fc-2', 'allow');
    let rec = h.loadTurnRecord('sess-order');
    expect(rec?.awaiting_approval?.map((e) => e.function_call_id)).toEqual(['fc-1']);
    expect(rec?.work?.executed['fc-2']).toBeDefined();
    expect(rec?.state).toBe('function_awaiting_approval');

    await h.resolveApproval('sess-order', 'fc-1', 'allow');
    rec = h.loadTurnRecord('sess-order');
    expect(rec?.awaiting_approval).toEqual([]);
    expect(rec?.state).toBe('assistant_streaming');
    expect(rec?.work).toBeUndefined();
  });

  it('denies one pending call without affecting an unresolved sibling', async () => {
    const h = createParallelApprovalHarness();
    vi.spyOn(agentTriggerModule, 'dispatchWithHook')
      .mockResolvedValueOnce({
        kind: 'pending',
        held_by: 'approval::gate',
        pending_timeout_ms: 1_800_000,
      })
      .mockResolvedValueOnce({
        kind: 'pending',
        held_by: 'approval::gate',
        pending_timeout_ms: 1_800_000,
      });

    h.seedExecute(
      'sess-deny',
      makeAssistantWithCalls([
        { id: 'fc-1', functionId: 'shell::run' },
        { id: 'fc-2', functionId: 'shell::run' },
      ]),
    );
    await h.runExecute('sess-deny');

    await h.resolveApproval('sess-deny', 'fc-1', 'deny', 'operator rejected');

    const rec = h.loadTurnRecord('sess-deny');
    expect(rec?.state).toBe('function_awaiting_approval');
    expect(rec?.awaiting_approval?.map((e) => e.function_call_id)).toEqual(['fc-2']);
    expect(rec?.work?.executed['fc-1']?.is_error).toBe(true);
    expect(rec?.work?.executed['fc-1']?.result.details).toMatchObject({
      status: 'denied',
      denied_by: 'user',
      reason: 'operator rejected',
    });
    expect(rec?.work?.executed['fc-1']?.result.content[0]).toMatchObject({
      text: 'operator rejected',
    });
    expect(rec?.work?.executed['fc-2']).toBeUndefined();
  });

  it('is idempotent when the same decision wake is delivered twice', async () => {
    const h = createParallelApprovalHarness();
    vi.spyOn(agentTriggerModule, 'dispatchWithHook').mockResolvedValueOnce({
      kind: 'pending',
      held_by: 'approval::gate',
      pending_timeout_ms: 1_800_000,
    });

    h.seedExecute('sess-dup', makeAssistantWithCalls([{ id: 'fc-1', functionId: 'shell::run' }]));
    await h.runExecute('sess-dup');

    await h.resolveApproval('sess-dup', 'fc-1', 'allow');
    const endsAfterFirst = executionEvents(h.emitted, 'function_execution_end', 'fc-1').length;

    await h.resolveApproval('sess-dup', 'fc-1', 'allow');
    const rec = h.loadTurnRecord('sess-dup');

    expect(rec?.awaiting_approval).toEqual([]);
    expect(executionEvents(h.emitted, 'function_execution_end', 'fc-1')).toHaveLength(
      endsAfterFirst,
    );
  });

  it('resolves two approvals fired in parallel without double-executing or double-finalizing', async () => {
    const h = createParallelApprovalHarness();
    vi.spyOn(agentTriggerModule, 'dispatchWithHook')
      .mockResolvedValueOnce({
        kind: 'pending',
        held_by: 'approval::gate',
        pending_timeout_ms: 1_800_000,
      })
      .mockResolvedValueOnce({
        kind: 'pending',
        held_by: 'approval::gate',
        pending_timeout_ms: 1_800_000,
      });

    h.seedExecute(
      'sess-par',
      makeAssistantWithCalls([
        { id: 'fc-1', functionId: 'shell::run' },
        { id: 'fc-2', functionId: 'shell::run' },
      ]),
    );
    await h.runExecute('sess-par');
    expect(h.loadTurnRecord('sess-par')?.awaiting_approval?.map((e) => e.function_call_id)).toEqual(
      ['fc-1', 'fc-2'],
    );

    await Promise.all([
      h.resolveApproval('sess-par', 'fc-1', 'allow'),
      h.resolveApproval('sess-par', 'fc-2', 'allow'),
    ]);

    const turnEnds = h.emitted.filter((e) => e.type === 'turn_end');
    expect(executionEvents(h.emitted, 'function_execution_end', 'fc-1')).toHaveLength(1);
    expect(executionEvents(h.emitted, 'function_execution_end', 'fc-2')).toHaveLength(1);
    expect(turnEnds).toHaveLength(1);
  });

  it('drains a sibling approved mid-execution even when its own wake is dropped', async () => {
    const h = createParallelApprovalHarness();
    vi.spyOn(agentTriggerModule, 'dispatchWithHook')
      .mockResolvedValueOnce({
        kind: 'pending',
        held_by: 'approval::gate',
        pending_timeout_ms: 1_800_000,
      })
      .mockResolvedValueOnce({
        kind: 'pending',
        held_by: 'approval::gate',
        pending_timeout_ms: 1_800_000,
      });
    vi.spyOn(agentTriggerModule, 'triggerFunctionCall').mockImplementation(async (_iii, call) => {
      if (call.id === 'fc-driver') {
        const turn_id = h.loadTurnRecord('sess-drain')?.turn_id ?? 't_x';
        h.stateStore.set('function_resolutions/sess-drain/fc-late', {
          turn_id,
          action: 'execute',
          resolved_at: 1,
        });
      }
      return {
        content: [{ type: 'text' as const, text: `${call.id}-ok` }],
        details: {},
        terminate: false,
      };
    });

    h.seedExecute(
      'sess-drain',
      makeAssistantWithCalls([
        { id: 'fc-late', functionId: 'shell::run' },
        { id: 'fc-driver', functionId: 'shell::run' },
      ]),
    );
    await h.runExecute('sess-drain');
    expect(
      h.loadTurnRecord('sess-drain')?.awaiting_approval?.map((e) => e.function_call_id),
    ).toEqual(['fc-late', 'fc-driver']);

    await h.resolveApproval('sess-drain', 'fc-driver', 'allow');

    const rec = h.loadTurnRecord('sess-drain');
    expect(executionEvents(h.emitted, 'function_execution_end', 'fc-driver')).toHaveLength(1);
    expect(executionEvents(h.emitted, 'function_execution_end', 'fc-late')).toHaveLength(1);
    expect(rec?.awaiting_approval).toEqual([]);
    expect(rec?.state).toBe('assistant_streaming');
  });

  it('re-enqueues a follow-up wake when a resolved call leaves siblings pending', async () => {
    const h = createParallelApprovalHarness();
    vi.spyOn(agentTriggerModule, 'dispatchWithHook')
      .mockResolvedValueOnce({
        kind: 'pending',
        held_by: 'approval::gate',
        pending_timeout_ms: 1_800_000,
      })
      .mockResolvedValueOnce({
        kind: 'pending',
        held_by: 'approval::gate',
        pending_timeout_ms: 1_800_000,
      });

    h.seedExecute(
      'sess-reenqueue',
      makeAssistantWithCalls([
        { id: 'fc-1', functionId: 'shell::run' },
        { id: 'fc-2', functionId: 'shell::run' },
      ]),
    );
    await h.runExecute('sess-reenqueue');

    const before = wakeEnqueues(h, 'sess-reenqueue');
    await h.resolveApproval('sess-reenqueue', 'fc-1', 'allow');

    const rec = h.loadTurnRecord('sess-reenqueue');
    expect(rec?.awaiting_approval?.map((e) => e.function_call_id)).toEqual(['fc-2']);
    expect(wakeEnqueues(h, 'sess-reenqueue')).toBeGreaterThanOrEqual(before + 2);
  });

  it('wakes the parked turn via harness::function::resolve and deletes the consumed row', async () => {
    const h = createParallelApprovalHarness();
    vi.spyOn(agentTriggerModule, 'dispatchWithHook').mockResolvedValueOnce({
      kind: 'pending',
      held_by: 'approval::gate',
      pending_timeout_ms: 1_800_000,
    });

    h.seedExecute('sess-wake', makeAssistantWithCalls([{ id: 'fc-1', functionId: 'shell::run' }]));
    await h.runExecute('sess-wake');

    expect(h.loadTurnRecord('sess-wake')?.state).toBe('function_awaiting_approval');

    const out = await h.resolveApproval('sess-wake', 'fc-1', 'allow');

    expect(out).toEqual({ resolved: true, turn_resumed: true });
    // The resolution row is consumed and deleted — the scope never accumulates.
    expect(h.stateStore.has('function_resolutions/sess-wake/fc-1')).toBe(false);
    expect(h.loadTurnRecord('sess-wake')?.state).toBe('assistant_streaming');
    expect(h.loadTurnRecord('sess-wake')?.work).toBeUndefined();
  });

  it('answers {resolved: false} for an unknown call and a stale turn_id', async () => {
    const h = createParallelApprovalHarness();
    vi.spyOn(agentTriggerModule, 'dispatchWithHook').mockResolvedValueOnce({
      kind: 'pending',
      held_by: 'approval::gate',
      pending_timeout_ms: 1_800_000,
    });

    h.seedExecute('sess-miss', makeAssistantWithCalls([{ id: 'fc-1', functionId: 'shell::run' }]));
    await h.runExecute('sess-miss');

    // Unknown function_call_id.
    const unknown = await h.resolveApproval('sess-miss', 'fc-unknown', 'allow');
    expect(unknown).toEqual({ resolved: false });
    expect(h.loadTurnRecord('sess-miss')?.state).toBe('function_awaiting_approval');

    // Duplicate after the call settled.
    await h.resolveApproval('sess-miss', 'fc-1', 'allow');
    const dup = await h.resolveApproval('sess-miss', 'fc-1', 'allow');
    expect(dup).toEqual({ resolved: false });
  });
});
