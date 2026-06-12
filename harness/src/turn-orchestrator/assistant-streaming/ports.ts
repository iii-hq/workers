/**
 * Typed dependency ports and domain types for assistant_streaming.
 */

import { z } from 'zod';
import { logger } from '../../runtime/otel.js';
import type { ISdk } from '../../runtime/iii.js';
import type { AgentMessage, AssistantMessage } from '../../types/agent-message.js';
import type { AgentFunction } from '../../types/function.js';
import type { AssistantMessageEvent } from '../../types/stream-event.js';
import { AgentFunctionSchema } from '../../types/provider.js';
import { emit } from '../events.js';
import { runPreflight } from '../preflight.js';
import { streamProviderTurn } from '../provider-stream.js';
import type { RunRequest } from '../run-request.js';
import type { Model } from '../../models-catalog/types.js';
import { createTurnStatePorts, type TurnStatePorts } from '../state-runtime/ports.js';
import { isDuplicateAssistant } from '../state-runtime/transcript.js';

export type StreamContext = {
  session_id: string;
  /** Routed provider from provisioning; '' lets the router decide server-side. */
  provider: string;
  model: string;
  system_prompt: string;
  tools: AgentFunction[];
  messages: AgentMessage[];
  /** Optional reasoning/thinking level from the run request. Absent or 'off' = off. */
  thinking_level?: string;
  /**
   * Deterministic id for this turn's router request
   * (`${session_id}:${started_at_ms}`): `run::abort` recomputes it for
   * `router::abort`, and the router threads it to providers as the
   * credential-resolve dedup key.
   */
  request_id: string;
};

export type StreamTurnOutcome = {
  final: AssistantMessage | null;
  error: string | null;
  body_streamed: boolean;
};

export type AssistantRoute =
  | { kind: 'stopped'; reason: 'error' | 'aborted' }
  | { kind: 'function_execute' }
  | { kind: 'end_turn' };

export function parseFunctionSchemas(raw: unknown[]): AgentFunction[] {
  return z.array(AgentFunctionSchema).parse(raw) as AgentFunction[];
}

export function hasFunctionCalls(asst: AssistantMessage): boolean {
  return asst.content.some((b) => b.type === 'function_call');
}

export function isErrorOrAborted(asst: AssistantMessage): boolean {
  return asst.stop_reason === 'error' || asst.stop_reason === 'aborted';
}

export type DeltaHandler = (
  partial: AssistantMessage,
  event: AssistantMessageEvent,
) => Promise<void>;

export type AssistantStreamingPorts = TurnStatePorts & {
  loadRunRequest(session_id: string): Promise<RunRequest>;
  runPreflight(
    session_id: string,
    messages: AgentMessage[],
    provider: string,
    model: string,
    model_meta?: Model,
  ): Promise<'ok' | 'compacted'>;
  streamTurn(
    ctx: StreamContext,
    onDelta: DeltaHandler,
  ): Promise<{ final: AssistantMessage | null; error: string | null }>;
  emitMessageUpdate(
    session_id: string,
    message: AssistantMessage,
    event: AssistantMessageEvent,
  ): Promise<void>;
  emitMessageComplete(
    session_id: string,
    message: AssistantMessage,
    body_streamed: boolean,
  ): Promise<void>;
  persistAssistantIfNew(
    session_id: string,
    asst: AssistantMessage,
    messages: AgentMessage[],
  ): Promise<void>;
};

export function createStreamingPorts(iii: ISdk): AssistantStreamingPorts {
  const base = createTurnStatePorts(iii);

  return {
    ...base,

    async runPreflight(session_id, messages, provider, model, model_meta) {
      return runPreflight(iii, session_id, messages, provider, model, model_meta);
    },

    async streamTurn(ctx, onDelta) {
      // 'off' is a run-request convention, not a router thinking level —
      // the documented contract is omission.
      const thinking = ctx.thinking_level && ctx.thinking_level !== 'off';
      const { final, error } = await streamProviderTurn(iii, {
        session_id: ctx.session_id,
        targetFn: 'router::chat',
        buildInput: (writerRef) => ({
          writer_ref: writerRef,
          request_id: ctx.request_id,
          model: ctx.model,
          ...(ctx.provider ? { provider: ctx.provider } : {}),
          system_prompt: ctx.system_prompt,
          messages: ctx.messages,
          tools: ctx.tools,
          ...(thinking ? { thinking_level: ctx.thinking_level } : {}),
          metadata: { session_id: ctx.session_id },
        }),
        onDelta,
      });
      return { final, error };
    },

    async emitMessageUpdate(session_id, message, event) {
      await emit(iii, session_id, {
        type: 'message_update',
        message,
        llm_event: event,
      });
    },

    async emitMessageComplete(session_id, message, body_streamed) {
      await emit(iii, session_id, {
        type: 'message_complete',
        message,
        body_streamed,
      });
    },

    async persistAssistantIfNew(session_id, asst, messages) {
      if (isDuplicateAssistant(messages, asst)) {
        logger.warn('finalizeAssistant: skipping duplicate assistant push (re-entry detected)', {
          session_id,
          timestamp: asst.timestamp,
        });
        return;
      }
      await base.appendMessages(session_id, [asst]);
    },
  };
}
