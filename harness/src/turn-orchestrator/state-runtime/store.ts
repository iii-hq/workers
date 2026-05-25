/**
 * Agent-scope turn FSM store. All `state::*` I/O for turn-orchestrator goes
 * through `createTurnStore`.
 */

import { z } from 'zod';
import { TriggerAction, type ISdk } from '../../runtime/iii.js';
import { stateGet, stateListValues, stateSet } from '../../runtime/state.js';
import { logger } from '../../runtime/otel.js';
import type { AgentMessage } from '../../types/agent-message.js';
import { MESSAGES_SCOPE, RUN_REQUEST_SCOPE, TURN_STATE_SCOPE } from '../state.js';
import { emit } from '../events.js';
import { type RunRequest, parseRunRequest } from '../run-request.js';
import { toView, type TurnStateView } from '../schemas.js';
import { mirrorMessagesToSessionTree } from '../session-tree-mirror.js';
import { type TurnState, type TurnStateRecord, parseTurnStateRecord } from '../state.js';

export const TURN_STEP_QUEUE = 'turn-step';

const NON_STEPABLE_STATES = new Set<TurnState>(['stopped', 'failed', 'function_awaiting_approval']);

/** True when a persisted turn_state transition should enqueue `turn::{newState}`. */
export function shouldWakeStep(previousState: TurnState | null, newState: TurnState): boolean {
  if (NON_STEPABLE_STATES.has(newState)) return false;
  if (previousState !== null && previousState === newState) return false;
  return true;
}

async function enqueueTurnStep(iii: ISdk, session_id: string, state: TurnState): Promise<void> {
  try {
    await iii.trigger({
      function_id: `turn::${state}`,
      payload: { session_id },
      action: TriggerAction.Enqueue({ queue: TURN_STEP_QUEUE }),
    });
  } catch (err) {
    logger.warn('wakeStep failed', { session_id, state, err: String(err) });
  }
}

export type TurnStore = {
  loadRecord(session_id: string): Promise<TurnStateRecord | null>;
  saveRecord(rec: TurnStateRecord, previous?: TurnStateRecord | null): Promise<void>;
  writeRecord(rec: TurnStateRecord): Promise<void>;
  loadMessages(session_id: string): Promise<AgentMessage[]>;
  saveMessages(session_id: string, messages: AgentMessage[]): Promise<void>;
  appendMessages(session_id: string, msgs: AgentMessage[]): Promise<void>;
  loadRunRequest(session_id: string): Promise<RunRequest>;
  saveRunRequest(session_id: string, request: RunRequest): Promise<void>;
  listTurnStateRecords(): Promise<TurnStateRecord[]>;
  wakeStep(session_id: string, state: TurnState): Promise<void>;
  wakeFromRecord(session_id: string): Promise<void>;
};

const FlatMessagesSchema = z
  .array(z.custom<AgentMessage>((v) => v != null && typeof v === 'object'))
  .catch([]);

/** @internal Exported for unit tests. */
export function parseFlatMessages(raw: unknown): AgentMessage[] {
  return FlatMessagesSchema.parse(raw ?? []);
}

const scopedGet = (iii: ISdk, scope: string, session_id: string) =>
  stateGet(iii, scope, session_id);
const scopedSet = (iii: ISdk, scope: string, session_id: string, value: unknown) =>
  stateSet(iii, scope, session_id, value);

async function emitTurnStateChanged(
  iii: ISdk,
  session_id: string,
  event_type: 'state:created' | 'state:updated',
  new_value: TurnStateView,
  old_value?: TurnStateView,
): Promise<void> {
  try {
    await emit(iii, session_id, {
      type: 'turn_state_changed',
      event_type,
      new_value,
      ...(old_value !== undefined && { old_value }),
    });
  } catch (err) {
    logger.warn('emitTurnStateChanged failed', {
      session_id,
      err: String(err),
    });
  }
}

async function persistRecord(
  iii: ISdk,
  rec: TurnStateRecord,
  previous?: TurnStateRecord | null,
): Promise<TurnStateRecord | null> {
  const result = await scopedSet(iii, TURN_STATE_SCOPE, rec.session_id, rec);
  const prev = previous !== undefined ? previous : parseTurnStateRecord(result?.old_value ?? null);

  await emitTurnStateChanged(
    iii,
    rec.session_id,
    prev == null ? 'state:created' : 'state:updated',
    toView(rec),
    prev != null ? toView(prev) : undefined,
  );

  return prev;
}

export function createTurnStore(iii: ISdk): TurnStore {
  return {
    async loadRecord(session_id) {
      return parseTurnStateRecord(await scopedGet(iii, TURN_STATE_SCOPE, session_id));
    },

    async listTurnStateRecords() {
      const values = await stateListValues<unknown>(iii, { scope: TURN_STATE_SCOPE });
      return values
        .map((value) => parseTurnStateRecord(value))
        .filter((rec): rec is TurnStateRecord => rec !== null);
    },

    async writeRecord(rec) {
      await scopedSet(iii, TURN_STATE_SCOPE, rec.session_id, rec);
    },

    async saveRecord(rec, previous) {
      const prev = await persistRecord(iii, rec, previous);
      if (shouldWakeStep(prev?.state ?? null, rec.state)) {
        await enqueueTurnStep(iii, rec.session_id, rec.state);
      }
    },

    wakeStep(session_id, state) {
      return enqueueTurnStep(iii, session_id, state);
    },

    async wakeFromRecord(session_id) {
      const rec = parseTurnStateRecord(await scopedGet(iii, TURN_STATE_SCOPE, session_id));
      if (!rec || rec.state === 'stopped' || rec.state === 'failed') return;
      await enqueueTurnStep(iii, session_id, rec.state);
    },

    async loadMessages(session_id) {
      return parseFlatMessages(await scopedGet(iii, MESSAGES_SCOPE, session_id));
    },

    async saveMessages(session_id, messages) {
      await scopedSet(iii, MESSAGES_SCOPE, session_id, messages);
      await mirrorMessagesToSessionTree(iii, session_id, messages);
    },

    async appendMessages(session_id, msgs) {
      const messages = parseFlatMessages(await scopedGet(iii, MESSAGES_SCOPE, session_id));
      await scopedSet(iii, MESSAGES_SCOPE, session_id, [...messages, ...msgs]);
      await mirrorMessagesToSessionTree(iii, session_id, [...messages, ...msgs]);
    },

    async saveRunRequest(session_id, request) {
      await scopedSet(iii, RUN_REQUEST_SCOPE, session_id, request);
    },

    async loadRunRequest(session_id) {
      return parseRunRequest(await scopedGet(iii, RUN_REQUEST_SCOPE, session_id));
    },
  };
}
