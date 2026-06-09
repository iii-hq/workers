/**
 * Route steering_check outcomes and apply transitions.
 *
 * COMPAT (release A of the steering_check removal): nothing transitions into
 * the `steering_check` state anymore — its routing is inlined into
 * assistant-streaming finalizeAssistantTurn and function-execute finalizeBatch.
 * This module stays registered for one release so queue messages / records
 * persisted in `steering_check` at deploy time drain instead of DLQ-wedging
 * the turn. Release B removes the state, schemas, and this registration.
 */

import {
  emitTurnEndOnce,
  endTurnForMaxTurns,
  maxTurnsReached,
  resumeToAssistantStreaming,
} from '../state-runtime/turn-end.js';
import type { SteeringCheckTurnRecord } from '../state.js';
import type { SteeringCheckPorts } from './ports.js';

export type SteeringRoute = 'continue_after_function' | 'end_turn';

export type SteeringCheckOutcome =
  | { kind: 'max_turns_reached' }
  | { kind: 'continue_after_function' }
  | { kind: 'end_turn' };

export function route(has_function_results: boolean): SteeringRoute {
  return has_function_results ? 'continue_after_function' : 'end_turn';
}

export async function processSteeringCheck(
  _ports: SteeringCheckPorts,
  rec: SteeringCheckTurnRecord,
): Promise<SteeringCheckOutcome> {
  const decision = route(rec.function_results.length > 0);

  if (decision === 'continue_after_function' && maxTurnsReached(rec)) {
    return { kind: 'max_turns_reached' };
  }

  switch (decision) {
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
      await endTurnForMaxTurns(ports, rec);
      return;
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
