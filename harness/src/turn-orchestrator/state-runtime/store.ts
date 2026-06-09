/**
 * Agent-scope turn FSM store. All `state::*` I/O for turn-orchestrator goes
 * through `createTurnStore`.
 */

import { TriggerAction, type ISdk } from '../../runtime/iii.js';
import { stateGet, stateSet } from '../../runtime/state.js';
import { logger } from '../../runtime/otel.js';
import type { AgentMessage } from '../../types/agent-message.js';
import { RUN_REQUEST_SCOPE, TURN_STATE_SCOPE } from '../state.js';
import { emit } from '../events.js';
import { type RunRequest, parseRunRequest } from '../run-request.js';
import { toView, type TurnStateView } from '../schemas.js';
import { type TurnState, type TurnStateRecord, parseTurnStateRecord } from '../state.js';
import { loadContextView } from './context-view.js';
import { persistedTrailingResultIds } from './transcript.js';

/**
 * Turn-step wakes go to the engine's `default` queue. NOTE: engine.config.yaml
 * defines a `turn-step` FIFO queue (session_id grouping, max_retries: 5,
 * concurrency: 1) that is currently ORPHANED — nothing enqueues to it.
 * Switching to it changes scheduling semantics for every step (per-session
 * ordering, retry bound) and is a deliberate follow-up, not a drive-by rename.
 */
export const TURN_STEP_QUEUE = 'default';

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
  ensureSession(session_id: string): Promise<void>;
  loadMessages(session_id: string): Promise<AgentMessage[]>;
  /**
   * Function_call_ids in the trailing `function_result` run, for batch dedup.
   * Reads the raw message list on the default leaf (not the compaction-rebuilt
   * window {@link loadMessages} returns), so it skips the paired
   * `session-tree::compactions` read — the trailing results live in the tail
   * either way.
   */
  loadTrailingResultIds(session_id: string): Promise<Set<string>>;
  appendMessages(session_id: string, msgs: AgentMessage[]): Promise<void>;
  loadRunRequest(session_id: string): Promise<RunRequest>;
  saveRunRequest(session_id: string, request: RunRequest): Promise<void>;
};

const scopedGet = (iii: ISdk, scope: string, session_id: string) =>
  stateGet(iii, scope, session_id);
const scopedSet = (iii: ISdk, scope: string, session_id: string, value: unknown) =>
  stateSet(iii, scope, session_id, value);

/**
 * Create the session-tree record if absent. Idempotent, but invoked exactly
 * once per run (at `run::start`) rather than wrapping every read/write — the
 * `run::start` gateway always precedes any turn-store load/append for a session,
 * so re-ensuring on each call was pure RPC overhead.
 */
async function ensureSessionTree(iii: ISdk, session_id: string): Promise<void> {
  await iii.trigger({
    function_id: 'session-tree::ensure',
    payload: { session_id },
    timeoutMs: 10_000,
  });
}

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

  const nextView = toView(rec);
  const prevView = prev != null ? toView(prev) : undefined;
  const viewChanged =
    prevView === undefined || JSON.stringify(prevView) !== JSON.stringify(nextView);

  if (viewChanged) {
    await emitTurnStateChanged(
      iii,
      rec.session_id,
      prev == null ? 'state:created' : 'state:updated',
      nextView,
      prevView,
    );
  }

  return prev;
}

export function createTurnStore(iii: ISdk): TurnStore {
  return {
    async loadRecord(session_id) {
      return parseTurnStateRecord(await scopedGet(iii, TURN_STATE_SCOPE, session_id));
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

    async ensureSession(session_id) {
      await ensureSessionTree(iii, session_id);
    },

    async loadMessages(session_id) {
      return loadContextView(iii, session_id);
    },

    async loadTrailingResultIds(session_id) {
      // Same response (and same `?? []` for an empty/new session) as
      // loadContextView, the sibling reader of session-tree::messages.
      const resp = await iii.trigger<unknown, { messages?: Array<{ message: AgentMessage }> }>({
        function_id: 'session-tree::messages',
        payload: { session_id },
      });
      const messages = (resp?.messages ?? []).map((e) => e.message);
      return persistedTrailingResultIds(messages);
    },

    async appendMessages(session_id, msgs) {
      if (msgs.length === 0) return;
      // One batch trigger chains all messages and resolves the active leaf once,
      // instead of a per-message session-tree::append (each of which re-read the
      // whole session to find the leaf).
      await iii.trigger({
        function_id: 'session-tree::append_batch',
        payload: { session_id, messages: msgs, parent_id: null },
        timeoutMs: 10_000,
      });
    },

    async saveRunRequest(session_id, request) {
      await scopedSet(iii, RUN_REQUEST_SCOPE, session_id, request);
    },

    async loadRunRequest(session_id) {
      return parseRunRequest(await scopedGet(iii, RUN_REQUEST_SCOPE, session_id));
    },
  };
}
