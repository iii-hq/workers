/**
 * Stream one provider turn, persist the assistant message, and route onward.
 *
 * The driver loop (integration.md §10): append an empty assistant entry with
 * a deterministic id before the provider stream starts, replace its content
 * via `session::update_message` per coalesced delta batch, and land the final
 * content with a last strict update. Step re-entry reuses the same entry
 * (idempotent append) instead of duplicating it.
 */

import { assistantEntryId, runKey } from '../../runtime/session.js';
import type { AssistantMessage } from '../../types/agent-message.js';
import { decide } from '../provider-router.js';
import { syntheticAssistant } from '../synthetic-assistant.js';
import { emitTurnEndOnce, transitionToFinishing } from '../state-runtime/turn-end.js';
import { enterFunctionExecute } from '../function-execute/run.js';
import { transitionTo, type AssistantStreamingTurnRecord } from '../state.js';
import { createDeltaCoalescer } from './coalesce-deltas.js';
import {
  hasFunctionCalls,
  isErrorOrAborted,
  parseFunctionSchemas,
  type AssistantRoute,
  type AssistantStreamingPorts,
  type StreamContext,
  type StreamTurnOutcome,
} from './ports.js';

export function beginTurn(rec: AssistantStreamingTurnRecord): void {
  rec.turn_count++;
  rec.turn_end_emitted = false;
  rec.assistant_body_streamed = false;
}

/** Deterministic session-manager entry id for this turn's assistant message. */
export function turnAssistantEntryId(rec: AssistantStreamingTurnRecord): string {
  return assistantEntryId(runKey(rec), rec.turn_count);
}

export async function prepareStreamContext(
  ports: AssistantStreamingPorts,
  rec: AssistantStreamingTurnRecord,
  assistantEntry: string,
): Promise<StreamContext> {
  const request = await ports.loadRunRequest(rec.session_id);
  // Exclude this turn's assistant entry: on step re-entry the placeholder
  // already sits on the path (possibly with partial content) and must not be
  // replayed to the provider as history.
  const loadOpts = { excludeEntryIds: [assistantEntry] };
  let messages = await ports.loadMessages(rec.session_id, loadOpts);
  const { provider, model, system_prompt, function_schemas, thinking_level } = request;
  const decision = decide({ provider, model });
  const tools = parseFunctionSchemas(function_schemas);
  const model_meta = rec.model_meta;

  if (
    (await ports.runPreflight(rec.session_id, messages, decision.provider, model, model_meta)) ===
    'compacted'
  ) {
    messages = await ports.loadMessages(rec.session_id, loadOpts);
  }

  return {
    session_id: rec.session_id,
    decision,
    system_prompt,
    tools,
    messages,
    ...(thinking_level ? { thinking_level } : {}),
    ...(model_meta ? { model_meta } : {}),
    resolution_key: rec.started_at_ms,
  };
}

export async function runStreamTurn(
  ports: AssistantStreamingPorts,
  session_id: string,
  assistantEntry: string,
  ctx: StreamContext,
): Promise<StreamTurnOutcome> {
  let body_streamed = false;

  // Coalesce consecutive same-type provider deltas into one update to cut the
  // per-token RPC explosion on the streaming path. Each flush replaces the
  // session entry's content (durable; fires session::message_updated — the
  // live token surface consumers render from). Discrete events flush through
  // 1:1; the final flush below guarantees the tail.
  const coalescer = createDeltaCoalescer(async (partial, _event) => {
    await ports.updateAssistantContent(session_id, assistantEntry, partial.content, {
      tolerant: true,
    });
  });

  const { final, error } = await ports.streamTurn(ctx, async (partial, event) => {
    if (event.type === 'text_delta' || event.type === 'thinking_delta') {
      body_streamed = true;
    }
    await coalescer.onEvent(partial, event);
  });
  await coalescer.flush();

  return { final, error, body_streamed };
}

export function resolveAssistantMessage(
  outcome: StreamTurnOutcome,
  decision: StreamContext['decision'],
): AssistantMessage {
  if (outcome.final) return outcome.final;

  const reason = outcome.error ?? 'provider channel closed without final';
  return syntheticAssistant({
    stop_reason: 'error',
    text: reason,
    provider: decision.provider,
    model: decision.model,
  });
}

export function routeAssistantTurn(asst: AssistantMessage): AssistantRoute {
  if (isErrorOrAborted(asst)) {
    return {
      kind: 'stopped',
      reason: asst.stop_reason === 'aborted' ? 'aborted' : 'error',
    };
  }
  if (hasFunctionCalls(asst)) {
    return { kind: 'function_execute' };
  }
  return { kind: 'end_turn' };
}

export async function finalizeAssistantTurn(
  ports: AssistantStreamingPorts,
  rec: AssistantStreamingTurnRecord,
  asst: AssistantMessage,
  assistantEntry: string,
): Promise<void> {
  // Strict final update: the durable transcript must hold the full message
  // (or the synthetic error text) before completion is announced. Covers the
  // error/aborted routes too — partial output survives for reload.
  await ports.updateAssistantContent(rec.session_id, assistantEntry, asst.content);

  await ports.emitMessageComplete(rec.session_id, asst, rec.assistant_body_streamed === true);

  const route = routeAssistantTurn(asst);

  if (route.kind === 'stopped') {
    await emitTurnEndOnce(ports, rec, asst);
    transitionToFinishing(rec);
    return;
  }

  if (route.kind === 'function_execute') {
    rec.function_results = [];
    enterFunctionExecute(rec, asst);
    transitionTo(rec, 'function_execute');
    return;
  }

  await emitTurnEndOnce(ports, rec, asst);
  transitionToFinishing(rec);
}

export async function runAssistantStreaming(
  ports: AssistantStreamingPorts,
  rec: AssistantStreamingTurnRecord,
): Promise<void> {
  beginTurn(rec);
  const assistantEntry = turnAssistantEntryId(rec);
  const ctx = await prepareStreamContext(ports, rec, assistantEntry);
  await ports.appendAssistantPlaceholder(rec.session_id, assistantEntry, ctx.decision, {
    turn: rec.turn_count,
  });
  const outcome = await runStreamTurn(ports, rec.session_id, assistantEntry, ctx);
  // When the provider died without a final, resolveAssistantMessage builds a
  // synthetic error assistant; its text lands on the entry via the strict
  // final update in finalizeAssistantTurn (no separate delta emission).
  const asst = resolveAssistantMessage(outcome, ctx.decision);
  rec.last_assistant = asst;
  rec.assistant_body_streamed = outcome.body_streamed;

  await finalizeAssistantTurn(ports, rec, asst, assistantEntry);
}
