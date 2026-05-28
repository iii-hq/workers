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

describe('parallel approval e2e', () => {
  it('dispatches later calls while earlier ones park without blocking the batch', async () => {
    const h = createParallelApprovalHarness();
    vi.spyOn(agentTriggerModule, 'dispatchWithHook')
      .mockResolvedValueOnce({ kind: 'pending' })
      .mockResolvedValueOnce({
        kind: 'result',
        result: {
          content: [{ type: 'text' as const, text: 'fc-2-ok' }],
          details: {},
          terminate: false,
        },
      })
      .mockResolvedValueOnce({ kind: 'pending' });

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
      .mockResolvedValueOnce({ kind: 'pending' })
      .mockResolvedValueOnce({
        kind: 'result',
        result: { content: [{ type: 'text' as const, text: 'ok' }], details: {}, terminate: false },
      })
      .mockResolvedValueOnce({ kind: 'pending' });

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
      .mockResolvedValueOnce({ kind: 'pending' })
      .mockResolvedValueOnce({ kind: 'pending' });

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
    expect(rec?.state).toBe('steering_check');
    expect(rec?.work).toBeUndefined();
  });

  it('denies one pending call without affecting an unresolved sibling', async () => {
    const h = createParallelApprovalHarness();
    vi.spyOn(agentTriggerModule, 'dispatchWithHook')
      .mockResolvedValueOnce({ kind: 'pending' })
      .mockResolvedValueOnce({ kind: 'pending' });

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
      approval_denied: true,
      decision: 'deny',
      reason: 'operator rejected',
    });
    expect(rec?.work?.executed['fc-2']).toBeUndefined();
  });

  it('is idempotent when the same decision wake is delivered twice', async () => {
    const h = createParallelApprovalHarness();
    vi.spyOn(agentTriggerModule, 'dispatchWithHook').mockResolvedValueOnce({ kind: 'pending' });

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
      .mockResolvedValueOnce({ kind: 'pending' })
      .mockResolvedValueOnce({ kind: 'pending' });

    h.seedExecute(
      'sess-par',
      makeAssistantWithCalls([
        { id: 'fc-1', functionId: 'shell::run' },
        { id: 'fc-2', functionId: 'shell::run' },
      ]),
    );
    await h.runExecute('sess-par');
    expect(
      h.loadTurnRecord('sess-par')?.awaiting_approval?.map((e) => e.function_call_id),
    ).toEqual(['fc-1', 'fc-2']);

    // Both prompts approved concurrently: two approval::resolve writes whose
    // wakes interleave on the shared turn_state record. The sequential tests
    // never exercise this. runTransition has no optimistic-concurrency guard
    // (saveRecord is an unconditional overwrite), so both wakes load the same
    // parked record, each executes EVERY call, and each finalizes the batch —
    // duplicating side effects and emitting turn_end multiple times.
    await Promise.all([
      h.resolveApproval('sess-par', 'fc-1', 'allow'),
      h.resolveApproval('sess-par', 'fc-2', 'allow'),
    ]);

    const turnEnds = h.emitted.filter((e) => e.type === 'turn_end');
    expect(executionEvents(h.emitted, 'function_execution_end', 'fc-1')).toHaveLength(1);
    expect(executionEvents(h.emitted, 'function_execution_end', 'fc-2')).toHaveLength(1);
    expect(turnEnds).toHaveLength(1);
  });

  it('persists the decision and wakes function_awaiting_approval via approval::resolve', async () => {
    const h = createParallelApprovalHarness();
    vi.spyOn(agentTriggerModule, 'dispatchWithHook').mockResolvedValueOnce({ kind: 'pending' });

    h.seedExecute('sess-wake', makeAssistantWithCalls([{ id: 'fc-1', functionId: 'shell::run' }]));
    await h.runExecute('sess-wake');

    expect(h.loadTurnRecord('sess-wake')?.state).toBe('function_awaiting_approval');

    await h.resolveApproval('sess-wake', 'fc-1', 'allow');

    expect(h.stateStore.get('approvals/sess-wake/fc-1')).toEqual({
      decision: 'allow',
      reason: null,
    });
    expect(h.loadTurnRecord('sess-wake')?.state).toBe('steering_check');
    expect(h.loadTurnRecord('sess-wake')?.work).toBeUndefined();
  });
});
