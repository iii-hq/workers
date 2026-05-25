/**
 * Typed dependency ports for steering_check.
 */

import type { ISdk } from '../../runtime/iii.js';
import type { AgentEvent } from '../../types/agent-event.js';
import type { AgentMessage } from '../../types/agent-message.js';
import { emit } from '../events.js';
import {
  createTurnStatePorts,
  type TurnStatePorts,
} from '../state-runtime/ports.js';

/** Decode session-inbox drain responses. */
export function parseDrainItems(resp: unknown): AgentMessage[] {
  if (resp && typeof resp === 'object' && Array.isArray((resp as { items?: unknown }).items)) {
    return (resp as { items: AgentMessage[] }).items;
  }
  return [];
}

export type SteeringCheckPorts = TurnStatePorts & {
  drainInbox(name: 'steering' | 'followup', session_id: string): Promise<AgentMessage[]>;
  emit(session_id: string, event: AgentEvent): Promise<void>;
};

export function createSteeringCheckPorts(iii: ISdk): SteeringCheckPorts {
  const base = createTurnStatePorts(iii);

  return {
    ...base,

    async drainInbox(name, session_id) {
      try {
        const resp = await iii.trigger<unknown, unknown>({
          function_id: 'session-inbox::drain',
          payload: { name, session_id },
        });
        return parseDrainItems(resp);
      } catch {
        return [];
      }
    },

    emit(session_id, event) {
      return emit(iii, session_id, event);
    },
  };
}
