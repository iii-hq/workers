/**
 * Shared dependency ports for turn FSM state handlers.
 */

import { emit } from '../events.js';
import type { RunRequest } from '../run-request.js';
import type { ISdk } from '../../runtime/iii.js';
import type { ModelContextLimit } from '../../types/agent-event.js';
import type { AgentMessage, FunctionResultMessage } from '../../types/agent-message.js';
import { transitionTo, type TurnStateRecord } from '../state.js';
import { createTurnStore, type TurnStore } from './store.js';

export type TurnStatePorts = {
  loadMessages(session_id: string): Promise<AgentMessage[]>;
  appendMessages(session_id: string, msgs: AgentMessage[]): Promise<void>;
  checkpoint(rec: TurnStateRecord): Promise<void>;
  loadRunRequest(session_id: string): Promise<RunRequest>;
  saveRunRequest(session_id: string, request: RunRequest): Promise<void>;
  emitTurnEnd(
    session_id: string,
    message: AgentMessage,
    function_results: FunctionResultMessage[],
    model_limit?: ModelContextLimit,
  ): Promise<void>;
  finishSession(rec: TurnStateRecord): Promise<void>;
};

export function createTurnStatePorts(iii: ISdk, store?: TurnStore): TurnStatePorts {
  const s = store ?? createTurnStore(iii);

  return {
    loadMessages(session_id) {
      return s.loadMessages(session_id);
    },

    appendMessages(session_id, msgs) {
      return s.appendMessages(session_id, msgs);
    },

    checkpoint(rec) {
      return s.writeRecord(rec);
    },

    loadRunRequest(session_id) {
      return s.loadRunRequest(session_id);
    },

    saveRunRequest(session_id, request) {
      return s.saveRunRequest(session_id, request);
    },

    async emitTurnEnd(session_id, message, function_results, model_limit) {
      await emit(iii, session_id, {
        type: 'turn_end',
        message,
        function_results,
        ...(model_limit ? { model_limit } : {}),
      });
    },

    async finishSession(rec) {
      await emit(iii, rec.session_id, { type: 'agent_end', messages: [] });
      transitionTo(rec, 'stopped');
    },
  };
}
