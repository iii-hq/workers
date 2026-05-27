/**
 * Drain inboxes, route steering_check outcomes, and apply transitions.
 */

import type { AgentMessage } from '../../types/agent-message.js';
import { syntheticAssistant } from '../synthetic-assistant.js';
import { emitTurnEndOnce, resumeToAssistantStreaming } from '../state-runtime/turn-end.js';
import type { SteeringCheckTurnRecord } from '../state.js';
import type { SteeringCheckPorts } from './ports.js';

export type SteeringRoute = 'steering' | 'followup' | 'continue_after_function' | 'end_turn';

export type SteeringCheckOutcome =
  | { kind: 'max_turns_reached' }
  | { kind: 'resume_with_inbox'; inbox: AgentMessage[] }
  | { kind: 'continue_after_function' }
  | { kind: 'end_turn' };

export function route(
  has_steering: boolean,
  has_followup: boolean,
  has_function_results: boolean,
): SteeringRoute {
  if (has_steering) return 'steering';
  if (has_followup) return 'followup';
  if (has_function_results) return 'continue_after_function';
  return 'end_turn';
}

function maxTurnsReached(rec: SteeringCheckTurnRecord): boolean {
  return rec.max_turns !== undefined && rec.turn_count >= rec.max_turns;
}

async function endForMaxTurns(
  ports: SteeringCheckPorts,
  rec: SteeringCheckTurnRecord,
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
  await ports.finishSession(rec);
}

export async function processSteeringCheck(
  ports: SteeringCheckPorts,
  rec: SteeringCheckTurnRecord,
): Promise<SteeringCheckOutcome> {
  const steering = await ports.drainInbox('steering', rec.session_id);
  const followup = steering.length > 0 ? [] : await ports.drainInbox('followup', rec.session_id);

  const decision = route(steering.length > 0, followup.length > 0, rec.function_results.length > 0);

  if (
    (decision === 'steering' ||
      decision === 'followup' ||
      decision === 'continue_after_function') &&
    maxTurnsReached(rec)
  ) {
    return { kind: 'max_turns_reached' };
  }

  switch (decision) {
    case 'steering':
      return { kind: 'resume_with_inbox', inbox: steering };
    case 'followup':
      return { kind: 'resume_with_inbox', inbox: followup };
    case 'continue_after_function':
      return { kind: 'continue_after_function' };
    case 'end_turn':
      return { kind: 'end_turn' };
  }
}

export async function applySteeringCheckOutcome(
  ports: SteeringCheckPorts,
  rec: SteeringCheckTurnRecord,
  outcome: SteeringCheckOutcome,
): Promise<void> {
  switch (outcome.kind) {
    case 'max_turns_reached':
      await endForMaxTurns(ports, rec);
      return;
    case 'resume_with_inbox': {
      await emitTurnEndOnce(ports, rec);
      await ports.appendMessages(rec.session_id, outcome.inbox);
      resumeToAssistantStreaming(rec);
      return;
    }
    case 'continue_after_function':
      resumeToAssistantStreaming(rec);
      return;
    case 'end_turn':
      await emitTurnEndOnce(ports, rec);
      await ports.finishSession(rec);
      return;
  }
}

export async function runSteeringCheck(
  ports: SteeringCheckPorts,
  rec: SteeringCheckTurnRecord,
): Promise<void> {
  const outcome = await processSteeringCheck(ports, rec);
  await applySteeringCheckOutcome(ports, rec, outcome);
}
