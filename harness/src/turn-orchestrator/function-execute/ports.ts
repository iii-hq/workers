/**
 * Typed dependency ports for function_execute — production wiring and test doubles.
 */

import { z } from 'zod';
import type { DispatchResult } from '../agent-trigger.js';
import { dispatchWithHook, triggerFunctionCall } from '../agent-trigger.js';
import { emit } from '../events.js';
import type { ISdk } from '../../runtime/iii.js';
import type { FunctionCall, FunctionResult } from '../../types/function.js';
import {
  createTurnStatePorts,
  type TurnStatePorts,
} from '../state-runtime/ports.js';
import type { ExecutedCall } from './types.js';

const RoutingEnvelopeSchema = z
  .object({
    session_id: z.string(),
    function_call_id: z.string(),
    function_id: z.string(),
    function_call: z.object({
      id: z.string(),
      function_id: z.string(),
      arguments: z.unknown(),
    }),
  })
  .catchall(z.unknown());

type RoutingEnvelope = z.infer<typeof RoutingEnvelopeSchema>;

function baseArgs(arguments_: FunctionCall['arguments']): Record<string, unknown> {
  if (arguments_ && typeof arguments_ === 'object' && !Array.isArray(arguments_)) {
    return { ...(arguments_ as Record<string, unknown>) };
  }
  return { arguments: arguments_ };
}

/** Attach session + call identity to arguments for policy and target functions. */
export function withRoutingEnvelope(call: FunctionCall, session_id: string): FunctionCall {
  const envelope: RoutingEnvelope = {
    ...baseArgs(call.arguments),
    session_id,
    function_call_id: call.id,
    function_id: call.function_id,
    function_call: { id: call.id, function_id: call.function_id, arguments: call.arguments },
  };
  RoutingEnvelopeSchema.parse(envelope);
  return { id: call.id, function_id: call.function_id, arguments: envelope };
}

export type FunctionExecutePorts = TurnStatePorts & {
  emitStart(session_id: string, call: FunctionCall): Promise<void>;
  emitEnd(session_id: string, executed: ExecutedCall): Promise<void>;
  dispatch(call: FunctionCall, session_id: string): Promise<DispatchResult>;
  triggerPreApproved(call: FunctionCall): Promise<FunctionResult>;
};

function buildFunctionExecutionEnd(executed: ExecutedCall) {
  return {
    type: 'function_execution_end' as const,
    function_call_id: executed.call.id,
    function_id: executed.call.function_id,
    result: executed.result,
    is_error: executed.is_error,
    duration_ms: executed.duration_ms,
  };
}

export function createPorts(iii: ISdk): FunctionExecutePorts {
  const base = createTurnStatePorts(iii);

  return {
    ...base,

    async emitStart(session_id, call) {
      await emit(iii, session_id, {
        type: 'function_execution_start',
        function_call_id: call.id,
        function_id: call.function_id,
        args: call.arguments,
      });
    },

    async emitEnd(session_id, executed) {
      await emit(iii, session_id, buildFunctionExecutionEnd(executed));
    },

    async dispatch(call, session_id) {
      return dispatchWithHook(iii, withRoutingEnvelope(call, session_id));
    },

    async triggerPreApproved(call) {
      return triggerFunctionCall(iii, call);
    },
  };
}
