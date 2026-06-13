/**
 * Integration harness for parallel approval flows: real TurnStore +
 * runTransition + harness::function::resolve, simulated iii
 * state/streams, and a scriptable fake `approval::gate` bound to the
 * real pre_dispatch hook chain.
 */

import { vi } from 'vitest';
import { TransientError } from '../../src/turn-orchestrator/errors.js';
import { handleAwaitingApproval } from '../../src/turn-orchestrator/function-awaiting-approval/process.js';
import { handleExecute } from '../../src/turn-orchestrator/function-execute/process.js';
import { enterFunctionExecute } from '../../src/turn-orchestrator/function-execute/run.js';
import {
  handleFunctionResolve,
  type FunctionResolveResult,
} from '../../src/turn-orchestrator/function-resolve.js';
import {
  addPreDispatchBindingForTests,
  resetPreDispatchBindingsForTests,
} from '../../src/turn-orchestrator/hooks/registry.js';
import type { HookOutput } from '../../src/turn-orchestrator/hooks/types.js';
import { runTransition } from '../../src/turn-orchestrator/run-transition.js';
import {
  TURN_STATE_SCOPE,
  newRecord,
  type TurnStateRecord,
} from '../../src/turn-orchestrator/state.js';
import type { ISdk } from '../../src/runtime/iii.js';
import type { AgentEvent } from '../../src/types/agent-event.js';
import type { AssistantMessage } from '../../src/types/agent-message.js';
import { FakeSessionManager } from '../_helpers/fakeSessionManager.js';

export type ParallelApprovalHarness = {
  iii: ISdk;
  stateStore: Map<string, unknown>;
  emitted: AgentEvent[];
  sessions: FakeSessionManager;
  /** Calls the fake `approval::gate` received through the hook chain. */
  gateCalls: unknown[];
  /** Script the fake gate's next answers (default: hold). */
  setGateOutput(output: HookOutput | ((input: unknown) => HookOutput)): void;
  loadTurnRecord(session_id: string): TurnStateRecord | null;
  seedExecute(session_id: string, assistant: AssistantMessage): TurnStateRecord;
  runExecute(session_id: string): Promise<void>;
  resolveApproval(
    session_id: string,
    function_call_id: string,
    decision: 'allow' | 'deny',
    reason?: string | null,
  ): Promise<FunctionResolveResult>;
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
    await runTransition(iii, 'function_awaiting_approval', handleAwaitingApproval, payload, {
      serialize: true,
    });
  }
}

/**
 * Model the durable queue's TransientError retry: a wake that loses the
 * per-session lease re-runs after the holder releases. Yields between attempts
 * so the lease holder can make progress.
 */
async function runTurnStepWithRetry(
  iii: ISdk,
  function_id: string,
  session_id: string,
): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    try {
      await runTurnStep(iii, function_id, session_id);
      return;
    } catch (err) {
      if (err instanceof TransientError) {
        await flushMicrotasks();
        continue;
      }
      throw err;
    }
  }
}

export function createParallelApprovalHarness(): ParallelApprovalHarness {
  const stateStore = new Map<string, unknown>();
  const emitted: AgentEvent[] = [];
  const sessions = new FakeSessionManager();
  const gateCalls: unknown[] = [];
  let gateOutput: HookOutput | ((input: unknown) => HookOutput) = {
    decision: 'hold',
    pending_timeout_ms: 1_800_000,
  };

  // Bind the fake gate to the REAL hook chain (module-global registry).
  resetPreDispatchBindingsForTests();
  addPreDispatchBindingForTests({
    id: 'test-gate',
    function_id: 'approval::gate',
    config: { functions: ['*'], timeout_ms: 5_000, on_error: 'fail_closed' },
  });

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
        // Durable transcript writes (session::append for function results).
        const sessionResult = await sessions.handle(function_id, payload);
        if (sessionResult !== undefined) return sessionResult;

        if (function_id === 'approval::gate') {
          gateCalls.push(payload);
          return typeof gateOutput === 'function' ? gateOutput(payload) : gateOutput;
        }

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
          return { old_value, new_value };
        }

        if (function_id === 'state::delete') {
          const p = payload as { scope: string; key: string };
          stateStore.delete(`${p.scope}/${p.key}`);
          return {};
        }

        if (function_id === 'state::update') {
          // Atomic read-modify-write returning the prior value. Models the
          // `increment` op (event counter) and root-path `set` op (lease).
          const p = payload as {
            scope: string;
            key: string;
            ops?: Array<{ type: string; path?: string; by?: number; value?: unknown }>;
          };
          const storeKey = `${p.scope}/${p.key}`;
          const old_value = stateStore.has(storeKey)
            ? structuredClone(stateStore.get(storeKey))
            : null;
          let next: unknown = old_value;
          for (const op of p.ops ?? []) {
            if ((op.path ?? '') !== '') continue;
            if (op.type === 'increment') {
              next = (typeof next === 'number' ? next : 0) + (op.by ?? 1);
            } else if (op.type === 'set') {
              next = op.value;
            }
          }
          stateStore.set(storeKey, next);
          return { old_value, new_value: structuredClone(next) };
        }

        if (function_id === 'stream::set') {
          const p = payload as { stream_name?: string; data: AgentEvent };
          // Only agent::events carries event frames; turn_end now enqueues a
          // compaction wake (a separate function_id) rather than mirroring to a
          // second stream, so `emitted` stays a faithful one-entry-per-event log.
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
          await runTurnStepWithRetry(iii as unknown as ISdk, function_id, p.session_id);
          return null;
        }

        return null;
      },
    ),
  } as unknown as ISdk;

  function loadTurnRecord(session_id: string): TurnStateRecord | null {
    const raw = stateStore.get(`${TURN_STATE_SCOPE}/${session_id}`);
    return raw ? (structuredClone(raw) as TurnStateRecord) : null;
  }

  return {
    iii,
    stateStore,
    emitted,
    sessions,
    gateCalls,

    setGateOutput(output) {
      gateOutput = output;
    },

    loadTurnRecord,

    seedExecute(session_id: string, assistant: AssistantMessage): TurnStateRecord {
      const rec = newRecord(session_id);
      enterFunctionExecute(rec, assistant);
      rec.state = 'function_execute';
      stateStore.set(`${TURN_STATE_SCOPE}/${session_id}`, structuredClone(rec));
      return rec;
    },

    async runExecute(session_id: string): Promise<void> {
      await runTurnStep(iii, 'turn::function_execute', session_id);
    },

    /** Drive the real harness::function::resolve handler, like the gate worker does. */
    async resolveApproval(
      session_id: string,
      function_call_id: string,
      decision: 'allow' | 'deny',
      reason: null | string = null,
    ): Promise<FunctionResolveResult> {
      const rec = loadTurnRecord(session_id);
      if (!rec) throw new Error(`no turn record for ${session_id}`);
      const why = reason ?? 'Rejected by operator.';
      const target =
        rec.awaiting_approval?.find((e) => e.function_call_id === function_call_id)?.function_id ??
        'unknown';
      const payload =
        decision === 'allow'
          ? {
              session_id,
              turn_id: rec.turn_id,
              function_call_id,
              action: 'execute' as const,
            }
          : {
              session_id,
              turn_id: rec.turn_id,
              function_call_id,
              action: 'deliver' as const,
              is_error: true,
              content: [{ type: 'text', text: why }],
              details: {
                schema_version: 1,
                status: 'denied',
                denied_by: 'user',
                function_id: target,
                reason: why,
              },
            };
      const out = await handleFunctionResolve(iii, payload);
      await flushMicrotasks();
      return out;
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
