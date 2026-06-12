/**
 * Shared dependency ports for turn FSM state handlers.
 */

import { emit } from '../events.js';
import type { RunRequest } from '../run-request.js';
import type { ISdk } from '../../runtime/iii.js';
import type { ModelContextLimit } from '../../types/agent-event.js';
import { sessionSetStatus } from '../../runtime/session.js';
import type { AgentMessage, FunctionResultMessage } from '../../types/agent-message.js';
import { transitionTo, type TurnStateRecord } from '../state.js';
import {
  createTurnStore,
  type AppendOptions,
  type LoadMessagesOptions,
  type TurnStore,
} from './store.js';

export type TurnStatePorts = {
  loadMessages(session_id: string, opts?: LoadMessagesOptions): Promise<AgentMessage[]>;
  appendMessages(session_id: string, msgs: AgentMessage[], opts?: AppendOptions): Promise<void>;
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

/** Terminal session status for a finished run: error stop ⇒ error + reason. */
export function finishStatus(rec: TurnStateRecord): { status: 'done' | 'error'; reason?: string } {
  const last = rec.last_assistant;
  if (last && last.stop_reason === 'error') {
    return { status: 'error', reason: last.error_message ?? 'turn failed' };
  }
  return { status: 'done' };
}

export function createTurnStatePorts(iii: ISdk, store?: TurnStore): TurnStatePorts {
  const s = store ?? createTurnStore(iii);

  return {
    loadMessages(session_id, opts) {
      return s.loadMessages(session_id, opts);
    },

    appendMessages(session_id, msgs, opts) {
      return s.appendMessages(session_id, msgs, opts);
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
      // agent_end is a turn-end SIGNAL only. The transcript reaches the UI
      // incrementally and is re-read from session-manager on reload, so no
      // consumer reads agent_end.messages. Emit it empty instead of reloading
      // the whole session to fill an unused field.
      await emit(iii, rec.session_id, { type: 'agent_end', messages: [] });
      const { status, reason } = finishStatus(rec);
      await sessionSetStatus(iii, rec.session_id, status, reason);
      transitionTo(rec, 'stopped');
    },
  };
}
