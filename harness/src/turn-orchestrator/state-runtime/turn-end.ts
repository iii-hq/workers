/**
 * Shared turn-end helper for step outcome application.
 */

import {
  emptyAssistant,
  type AssistantMessage,
  type FunctionResultMessage,
} from '../../types/agent-message.js';
import type { TurnStateRecord } from '../state.js';

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
