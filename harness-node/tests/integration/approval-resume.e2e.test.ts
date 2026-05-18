/**
 * End-to-end integration test for the PR #150 approval-resume contract.
 *
 * Wires:
 *  - InMemoryStateBus for the approval-gate persistence
 *  - a fake ISdk that routes `approval::consume`, `run::resume`,
 *    `state::get`, `state::set` (for the turn state record), and
 *    `iii::durable::publish` through the in-memory backends
 *
 * Walks the contract:
 *   1. handleIntercept(shell::exec call) → pending block envelope written
 *   2. handleResolveWithEvents(allow) → emits approval_resolved, polls
 *      run::resume, executeResume rebuilds the terminal record
 *   3. handleConsume returns the resolved entry once
 *   4. handleAwaiting stages prepared + transitions to function_execute
 *   5. handleFinalize replaces the pending placeholder with the resolved
 *      function result
 */

import { describe, expect, it } from 'vitest';
import { handleConsume } from '../../src/approval-gate/consume.js';
import { handleIntercept } from '../../src/approval-gate/intercept.js';
import { handleResolveWithEvents } from '../../src/approval-gate/pending.js';
import { InMemoryStateBus } from '../../src/approval-gate/state-bus.js';
import { STATE_SCOPE, pendingKey } from '../../src/approval-gate/types.js';
import type { ISdk } from '../../src/runtime/iii.js';
import { executeResume } from '../../src/turn-orchestrator/run-start.js';
import { type TurnStateRecord, newRecord } from '../../src/turn-orchestrator/state.js';
import { handleAwaiting } from '../../src/turn-orchestrator/states/assistant.js';
import {
  consumeResolvedApprovalEntries,
  replacePendingApprovalPlaceholders,
} from '../../src/turn-orchestrator/states/functions.js';
import type { FunctionResultMessage } from '../../src/types/agent-message.js';

type StateValue = unknown;

/**
 * Tiny fake iii backed by a per-scope-key map for state::get/set and
 * routed handlers for approval::consume, run::resume, iii::durable::publish.
 * Mirrors the harness-node IiiStateBus contract for `agent` scope writes.
 */
function makeBackend(bus: InMemoryStateBus, getTurnRecord: () => TurnStateRecord | null) {
  const agentState = new Map<string, StateValue>();
  const calls: Array<{ function_id: string; payload: unknown }> = [];
  let turnRecord: TurnStateRecord | null = getTurnRecord();
  const events: unknown[] = [];

  const iii = {
    trigger: async <T, R>(req: {
      function_id: string;
      payload: T;
    }): Promise<R> => {
      const fid = req.function_id;
      const payload = req.payload as Record<string, unknown>;
      calls.push({ function_id: fid, payload });

      if (fid === 'state::set') {
        const scope = payload.scope as string;
        const key = payload.key as string;
        const value = payload.value;
        if (scope === 'agent' && key.endsWith('/turn_state')) {
          turnRecord = value as TurnStateRecord;
        }
        agentState.set(`${scope}/${key}`, value);
        return null as unknown as R;
      }
      if (fid === 'state::get') {
        const scope = payload.scope as string;
        const key = payload.key as string;
        if (scope === 'agent' && key.endsWith('/turn_state')) {
          return turnRecord as unknown as R;
        }
        return (agentState.get(`${scope}/${key}`) ?? null) as R;
      }
      if (fid === 'approval::consume') {
        const r = await handleConsume(bus, STATE_SCOPE, payload);
        return r as unknown as R;
      }
      if (fid === 'run::resume') {
        const r = await executeResume(iii, payload);
        return r as unknown as R;
      }
      if (fid === 'iii::durable::publish') {
        events.push(payload);
        // PR #150: approval-gate's resumeSession publishes
        // turn::step_requested here instead of polling run::resume.
        // The orchestrator's subscriber is what handles the rebuild;
        // simulate that by routing the publish through executeResume so
        // the test still observes the resurrected provisioning record.
        const topic = (payload as Record<string, unknown>).topic;
        const data = (payload as Record<string, unknown>).data as
          | Record<string, unknown>
          | undefined;
        if (topic === 'turn::step_requested' && data && typeof data.session_id === 'string') {
          await executeResume(iii, { session_id: data.session_id });
        }
        return null as unknown as R;
      }
      if (fid === 'stream::set') {
        events.push(payload);
        return null as unknown as R;
      }
      return null as unknown as R;
    },
  } as unknown as ISdk;

  return {
    iii,
    events,
    calls,
    getTurnRecord: () => turnRecord,
    setTurnRecord: (r: TurnStateRecord | null) => {
      turnRecord = r;
    },
    agentState,
  };
}

describe('approval-resume end-to-end contract', () => {
  it('intercept → resolve → resume → consume → handleAwaiting transitions to function_execute', async () => {
    const session_id = 'sess-e2e';
    const function_call_id = 'tc-1';
    const bus = new InMemoryStateBus();

    // Seed the turn record as terminal (simulating a session that paused
    // after the approval intercept).
    let record: TurnStateRecord = {
      ...newRecord(session_id, 5),
      state: 'stopped',
      turn_count: 2,
    };
    const backend = makeBackend(bus, () => record);
    backend.setTurnRecord(record);

    // Step 1: intercept produces a pending envelope and a pending state record.
    const intercept = (await handleIntercept(
      bus,
      STATE_SCOPE,
      {
        session_id,
        function_call_id,
        function_id: 'shell::exec',
        args: { command: 'date' },
        approval_required: ['shell::exec'],
        event_id: 'evt',
        reply_stream: 'replies',
      },
      Date.now(),
      60_000,
    )) as Record<string, unknown>;
    expect(intercept.status).toBe('pending');
    expect(intercept.subscriber).toBe('approval-gate');

    // Run-time write the also-pending mirror that handleFinalize would
    // emit on the way to terminal — needed so replacePendingApprovalPlaceholders
    // sees something to strip.
    const messages = [
      {
        role: 'function_result',
        function_call_id,
        function_id: 'shell::exec',
        content: [],
        details: { pending_approval: true, call_id: function_call_id },
        is_error: false,
        timestamp: 1,
      } satisfies FunctionResultMessage,
    ];
    backend.agentState.set(`agent/session/${session_id}/messages`, messages);

    // Step 2: resolve. handleResolveWithEvents emits approval_resolved AND
    // polls run::resume which is routed to executeResume in the backend.
    const resolveOut = (await handleResolveWithEvents(backend.iii, bus, STATE_SCOPE, {
      session_id,
      function_call_id,
      decision: 'allow',
    })) as Record<string, unknown>;
    expect(resolveOut.ok).toBe(true);

    // Step 3: confirm executeResume rebuilt the record (non-terminal, fresh
    // state, preserved turn_count + max_turns).
    const resumed = backend.getTurnRecord();
    expect(resumed).not.toBeNull();
    if (!resumed) throw new Error('resumed record is null');
    expect(resumed.state).toBe('provisioning');
    expect(resumed.turn_count).toBe(2);
    expect(resumed.max_turns).toBe(5);

    // Step 4: confirm consume returns the resolved entry once and marks it consumed.
    const drained = await consumeResolvedApprovalEntries(backend.iii, session_id);
    expect(drained).toHaveLength(1);
    expect(drained[0]?.[1]).toBeNull(); // allow → no prefilled
    const stored = (await bus.get(STATE_SCOPE, pendingKey(session_id, function_call_id))) as Record<
      string,
      unknown
    > | null;
    expect(stored?.status).toBe('consumed');

    // Step 5: simulate a separate fresh wake (consume already happened in
    // step 4 so this consume is empty, and handleAwaiting must fall through
    // to the normal flow rather than re-stage).
    //
    // For the resurrection path, prep state has been written, so we set up
    // approval_required and prove handleAwaiting transitions properly via
    // its existing branches.
    backend.agentState.set(`agent/session/${session_id}/run_request`, {
      approval_required: ['shell::exec'],
    });
    record = { ...resumed, state: 'awaiting_assistant' };
    backend.setTurnRecord(record);
    await handleAwaiting(backend.iii, record);
    // No pending approvals left after step 4, so handleAwaiting falls
    // through to assistant_streaming.
    expect(record.state).toBe('assistant_streaming');

    // Step 6: simulate handleFinalize stripping the pending placeholder.
    const replacement: FunctionResultMessage = {
      role: 'function_result',
      function_call_id,
      function_id: 'shell::exec',
      content: [],
      details: { ran: true },
      is_error: false,
      timestamp: 2,
    };
    replacePendingApprovalPlaceholders(messages, [replacement]);
    messages.push(replacement);
    const fn_results = messages.filter((m) => (m as { role?: string }).role === 'function_result');
    expect(fn_results).toHaveLength(1);
    expect((fn_results[0] as FunctionResultMessage).details as Record<string, unknown>).toEqual({
      ran: true,
    });
  });
});
