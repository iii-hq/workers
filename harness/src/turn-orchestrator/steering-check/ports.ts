/**
 * Typed dependency ports for steering_check.
 */

import type { ISdk } from '../../runtime/iii.js';
import type { AgentEvent } from '../../types/agent-event.js';
import { emit } from '../events.js';
import { createTurnStatePorts, type TurnStatePorts } from '../state-runtime/ports.js';

export type SteeringCheckPorts = TurnStatePorts & {
  emit(session_id: string, event: AgentEvent): Promise<void>;
};

export function createSteeringCheckPorts(iii: ISdk): SteeringCheckPorts {
  const base = createTurnStatePorts(iii);

  return {
    ...base,

    emit(session_id, event) {
      return emit(iii, session_id, event);
    },
  };
}
