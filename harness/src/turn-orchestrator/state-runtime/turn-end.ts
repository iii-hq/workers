/**
 * Shared turn-end helper for step outcome application.
 */

import type { AgentEvent } from '../../types/agent-event.js';
import {
  emptyAssistant,
  type AgentMessage,
  type AssistantMessage,
  type FunctionResultMessage,
} from '../../types/agent-message.js';
import { syntheticAssistant } from '../synthetic-assistant.js';
import { transitionTo, type TurnStateRecord } from '../state.js';

export type TurnEndEmitter = {
  emitTurnEnd(
    session_id: string,
    message: AssistantMessage,
    function_results: FunctionResultMessage[],
  ): Promise<void>;
};

export async function emitTurnEndOnce(
  ports: TurnEndEmitter,
  rec: TurnStateRecord,
  message?: AssistantMessage,
  function_results: FunctionResultMessage[] = [],
): Promise<void> {
  if (rec.turn_end_emitted) return;
  const last = message ?? rec.last_assistant ?? emptyAssistant();
  await ports.emitTurnEnd(rec.session_id, last, function_results);
  rec.turn_end_emitted = true;
}

export function resumeToAssistantStreaming(rec: TurnStateRecord): void {
  rec.function_results = [];
  transitionTo(rec, 'assistant_streaming');
}

/**
 * Route a terminating turn to the `finishing` step instead of emitting agent_end
 * inline. The turn's durable work (assistant/results/notice + the turn_end
 * stream event) is already committed by the enclosing step's saveRecord; the
 * finishing step then emits agent_end and advances to `stopped` from a clean
 * replayable boundary, so a crash before the save re-runs the work WITHOUT
 * consumers having seen a premature run-end.
 */
export function transitionToFinishing(rec: TurnStateRecord): void {
  transitionTo(rec, 'finishing');
}

export function maxTurnsReached(rec: TurnStateRecord): boolean {
  return rec.max_turns !== undefined && rec.turn_count >= rec.max_turns;
}

export type MaxTurnsEndPorts = TurnEndEmitter & {
  appendMessages(session_id: string, msgs: AgentMessage[]): Promise<void>;
  emit(session_id: string, event: AgentEvent): Promise<void>;
};

/**
 * End the loop at the max_turns cap: persist + surface a synthetic assistant
 * notice, emit turn_end (no-op when the step already emitted it), then route to
 * the `finishing` step to emit agent_end after the durable commit.
 */
export async function endTurnForMaxTurns(
  ports: MaxTurnsEndPorts,
  rec: TurnStateRecord,
): Promise<void> {
  const msg = syntheticAssistant({
    stop_reason: 'end',
    text: `loop stopped: max_turns (${rec.max_turns ?? 0}) reached`,
  });
  rec.last_assistant = msg;
  await ports.appendMessages(rec.session_id, [msg]);
  await ports.emit(rec.session_id, {
    type: 'message_complete',
    message: msg,
    body_streamed: false,
  });
  await emitTurnEndOnce(ports, rec, msg);
  transitionToFinishing(rec);
}
