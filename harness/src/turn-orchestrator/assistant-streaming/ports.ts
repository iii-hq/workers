/**
 * Typed dependency ports and domain types for assistant_streaming.
 */

import { z } from 'zod';
import type { Model } from '../../models-catalog/types.js';
import { logger } from '../../runtime/otel.js';
import type { ISdk } from '../../runtime/iii.js';
import { sessionAppendMessage, sessionUpdateMessage } from '../../runtime/session.js';
import {
  emptyAssistant,
  type AgentMessage,
  type AssistantMessage,
} from '../../types/agent-message.js';
import type { ContentBlock } from '../../types/content.js';
import type { AgentFunction } from '../../types/function.js';
import type { AssistantMessageEvent } from '../../types/stream-event.js';
import { AgentFunctionSchema } from '../../types/provider.js';
import { emit } from '../events.js';
import { runPreflight } from '../preflight.js';
import { buildInput, targetFunctionId, type RouteDecision } from '../provider-router.js';
import { streamProviderTurn } from '../provider-stream.js';
import type { RunRequest } from '../run-request.js';
import { createTurnStatePorts, type TurnStatePorts } from '../state-runtime/ports.js';

export type StreamContext = {
  session_id: string;
  decision: RouteDecision;
  system_prompt: string;
  tools: AgentFunction[];
  messages: AgentMessage[];
  /** Optional reasoning/thinking level from the run request. Absent = off. */
  thinking_level?: string;
  /** Turn's pre-resolved catalog entry, threaded to the provider. Absent = off. */
  model_meta?: Model;
  /** Turn's stable id (run start time), so the provider dedupes credential resolution. */
  resolution_key?: number;
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
  /**
   * Once per turn on `agent::events`. Consumers read the terminal
   * `stop_reason` (length/error/aborted notices) from it — the stored entry
   * can't carry it (update_message replaces content only). Streaming deltas
   * are NOT mirrored to agent::events anymore; the durable
   * `session::message_updated` feed is the live token surface.
   */
  emitMessageComplete(
    session_id: string,
    message: AssistantMessage,
    body_streamed: boolean,
  ): Promise<void>;
  /**
   * Append this turn's empty assistant entry before the provider stream
   * starts (integration.md §10). Idempotent on `entry_id`: a step re-entry
   * reuses the existing entry and keeps streaming into it.
   */
  appendAssistantPlaceholder(
    session_id: string,
    entry_id: string,
    decision: RouteDecision,
    origin: Record<string, unknown>,
  ): Promise<void>;
  /**
   * Replace the assistant entry's content (full snapshot so far). Tolerant
   * mode logs and continues — a dropped mid-stream batch only costs one
   * `message_updated` revision; the final strict update restores parity.
   */
  updateAssistantContent(
    session_id: string,
    entry_id: string,
    content: ContentBlock[],
    opts?: { tolerant?: boolean; origin?: Record<string, unknown> },
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
      const { final, error } = await streamProviderTurn(iii, {
        session_id: ctx.session_id,
        targetFn: targetFunctionId(ctx.decision),
        buildInput: (writerRef) =>
          buildInput(
            ctx.decision,
            writerRef,
            ctx.system_prompt,
            ctx.messages,
            ctx.tools,
            ctx.thinking_level,
            ctx.model_meta,
            ctx.resolution_key,
          ),
        onDelta,
      });
      return { final, error };
    },

    async emitMessageComplete(session_id, message, body_streamed) {
      await emit(iii, session_id, {
        type: 'message_complete',
        message,
        body_streamed,
      });
    },

    async appendAssistantPlaceholder(session_id, entry_id, decision, origin) {
      await sessionAppendMessage(iii, {
        session_id,
        message: emptyAssistant(decision.provider, decision.model),
        entry_id,
        origin,
      });
    },

    async updateAssistantContent(session_id, entry_id, content, opts) {
      try {
        await sessionUpdateMessage(iii, { session_id, entry_id, content, origin: opts?.origin });
      } catch (err) {
        if (!opts?.tolerant) throw err;
        logger.warn('session::update_message failed mid-stream; continuing', {
          session_id,
          entry_id,
          err: String(err),
        });
      }
    },
  };
}
