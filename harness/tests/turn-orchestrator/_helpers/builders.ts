/**
 * Shared message-builder helpers for turn-orchestrator tests.
 *
 * Three flavors:
 *   - `makeAssistant(calls)`           — bare `function_call` blocks (legacy unit-test shape).
 *   - `makeAssistantWithCalls(calls)`  — `agent_trigger`-wrapped calls (production wire shape).
 *   - `agentTriggerCall(id, fn, payload)` — single `agent_trigger` envelope block, for unit tests
 *     that want to mix one wrapped call into the bare-shape builder.
 */

import type { AssistantMessage } from '../../../src/types/agent-message.js';

type RawCall = { id: string; function_id: string; arguments?: unknown };
type AgentTriggerCallSpec = { id: string; functionId: string; payload?: unknown };

const ASSISTANT_SKELETON = {
  role: 'assistant' as const,
  stop_reason: 'function_call' as const,
  error_message: null,
  error_kind: null,
  usage: null,
  model: 'm',
  provider: 'p',
  timestamp: 1,
};

/** Build an `AssistantMessage` with raw `function_call` blocks (target function_id stays untouched). */
export function makeAssistant(calls: RawCall[] = []): AssistantMessage {
  return {
    ...ASSISTANT_SKELETON,
    content: calls.map((c) => ({
      type: 'function_call' as const,
      id: c.id,
      function_id: c.function_id,
      arguments: c.arguments ?? {},
    })),
  };
}

/** Build a single `agent_trigger`-wrapped function_call content block. */
export function agentTriggerCall(
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

/** Build an `AssistantMessage` whose calls are all wrapped in the `agent_trigger` envelope. */
export function makeAssistantWithCalls(calls: AgentTriggerCallSpec[]): AssistantMessage {
  return {
    ...ASSISTANT_SKELETON,
    content: calls.map((c) => agentTriggerCall(c.id, c.functionId, c.payload ?? {})),
  };
}
