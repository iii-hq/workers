/**
 * Integration harness for parallel approval flows: real TurnStore + runTransition,
 * simulated iii state/streams, and dispatchWithHook routing.
 */

import { vi } from 'vitest';
import { handleResolveRequest } from '../../src/approval-gate/resolve.js';
import { handleAwaitingApproval } from '../../src/turn-orchestrator/function-awaiting-approval/process.js';
import { handleExecute } from '../../src/turn-orchestrator/function-execute/process.js';
import { runTransition } from '../../src/turn-orchestrator/run-transition.js';
import {
  handleApprovalDecisionWrite,
  isApprovalDecisionWrite,
} from '../../src/turn-orchestrator/on-approval.js';
import { TURN_STATE_SCOPE, newRecord, type TurnStateRecord } from '../../src/turn-orchestrator/state.js';
import type { ISdk } from '../../src/runtime/iii.js';
import type { AgentEvent } from '../../src/types/agent-event.js';
import type { AssistantMessage } from '../../src/types/agent-message.js';

export type ParallelApprovalHarness = {
  iii: ISdk;
  stateStore: Map<string, unknown>;
  emitted: AgentEvent[];
  loadTurnRecord(session_id: string): TurnStateRecord | null;
  seedExecute(session_id: string, assistant: AssistantMessage): TurnStateRecord;
  runExecute(session_id: string): Promise<void>;
  resolveApproval(
    session_id: string,
    function_call_id: string,
    decision: 'allow' | 'deny',
    reason?: string | null,
  ): Promise<void>;
};

function makeAgentTriggerCall(
  id: string,
  functionId: string,
  payload: unknown = {},
): { type: 'function_call'; id: string; function_id: string; arguments: unknown } {
  return {
    type: 'function_call',
    id,
    function_id: 'agent_trigger',
    arguments: { function: functionId, payload },
  };
}

export function makeAssistantWithCalls(
  calls: Array<{ id: string; functionId: string; payload?: unknown }>,
): AssistantMessage {
  return {
    role: 'assistant',
    content: calls.map((c) => makeAgentTriggerCall(c.id, c.functionId, c.payload ?? {})),
    stop_reason: 'function_call',
    error_message: null,
    error_kind: null,
    usage: null,
    model: 'm',
    provider: 'p',
    timestamp: 1,
  };
}

async function flushMicrotasks(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
}

async function runTurnStep(iii: ISdk, function_id: string, session_id: string): Promise<void> {
  const payload = { session_id };
  if (function_id === 'turn::function_execute') {
    await runTransition(iii, 'function_execute', handleExecute, payload);
    return;
  }
  if (function_id === 'turn::function_awaiting_approval') {
    await runTransition(iii, 'function_awaiting_approval', handleAwaitingApproval, payload);
  }
}

export function createParallelApprovalHarness(): ParallelApprovalHarness {
  const stateStore = new Map<string, unknown>();
  const emitted: AgentEvent[] = [];
  let eventSeq = 0;

  const iii = {
    trigger: vi.fn(
      async ({
        function_id,
        payload,
        action,
      }: {
        function_id: string;
        payload: unknown;
        action?: unknown;
      }) => {
        if (function_id === 'state::get') {
          const p = payload as { scope: string; key: string };
          const v = stateStore.get(`${p.scope}/${p.key}`);
          return v === undefined ? null : structuredClone(v);
        }

        if (function_id === 'state::set') {
          const p = payload as { scope: string; key: string; value: unknown };
          const storeKey = `${p.scope}/${p.key}`;
          const old_value = stateStore.has(storeKey)
            ? structuredClone(stateStore.get(storeKey))
            : null;
          const new_value = structuredClone(p.value);
          stateStore.set(storeKey, new_value);
          const event = {
            event_type: old_value == null ? 'state:created' : 'state:updated',
            scope: p.scope,
            key: p.key,
            old_value,
            new_value,
            message_type: 'state',
          };
          if (p.scope === 'approvals' && isApprovalDecisionWrite(event)) {
            await handleApprovalDecisionWrite(iii as unknown as ISdk, event);
          }
          return { old_value, new_value };
        }

        if (function_id === 'state::update') {
          eventSeq += 1;
          return { old_value: eventSeq - 1 };
        }

        if (function_id === 'stream::set') {
          const p = payload as { data: AgentEvent };
          emitted.push(p.data);
          return null;
        }

        if (function_id === 'shell::run') {
          return {
            content: [{ type: 'text', text: 'ok' }],
            details: {},
            terminate: false,
          };
        }

        if (function_id.startsWith('turn::') && action != null) {
          const p = payload as { session_id: string };
          await runTurnStep(iii as unknown as ISdk, function_id, p.session_id);
          return null;
        }

        return null;
      },
    ),
  } as unknown as ISdk;

  return {
    iii,
    stateStore,
    emitted,

    loadTurnRecord(session_id: string): TurnStateRecord | null {
      const raw = stateStore.get(`${TURN_STATE_SCOPE}/${session_id}`);
      return raw ? (structuredClone(raw) as TurnStateRecord) : null;
    },

    seedExecute(session_id: string, assistant: AssistantMessage): TurnStateRecord {
      const rec = newRecord(session_id);
      rec.state = 'function_execute';
      rec.last_assistant = assistant;
      stateStore.set(`${TURN_STATE_SCOPE}/${session_id}`, structuredClone(rec));
      return rec;
    },

    async runExecute(session_id: string): Promise<void> {
      await runTurnStep(iii, 'turn::function_execute', session_id);
    },

    async resolveApproval(
      session_id: string,
      function_call_id: string,
      decision: 'allow' | 'deny',
      reason: null | string = null,
    ): Promise<void> {
      const out = await handleResolveRequest(iii, {
        session_id,
        function_call_id,
        decision,
        reason,
      });
      if (!out.ok) throw new Error(`approval::resolve failed: ${out.error}`);
      await flushMicrotasks();
    },
  };
}

export function executionEvents(
  emitted: AgentEvent[],
  type: 'function_execution_start' | 'function_execution_end',
  function_call_id?: string,
): AgentEvent[] {
  return emitted.filter((event) => {
    if (event.type !== type) return false;
    if (!function_call_id) return true;
    return 'function_call_id' in event && event.function_call_id === function_call_id;
  });
}
