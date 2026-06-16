/**
 * Agent-scope turn FSM store. All `state::*` I/O for turn-orchestrator goes
 * through `createTurnStore`.
 */

import { TriggerAction, type ISdk } from '../../runtime/iii.js';
import { createState, stateGet, stateSet } from '../../runtime/state.js';
import { logger } from '../../runtime/otel.js';
import {
  functionResultEntryId,
  sessionAppendMessage,
  sessionEnsure,
} from '../../runtime/session.js';
import type { AgentMessage } from '../../types/agent-message.js';
import { RUN_REQUEST_SCOPE, TURN_STATE_SCOPE } from '../state.js';
import { emit } from '../events.js';
import { type RunRequest, defaultRunRequest } from '../run-request.js';
import { toView, type TurnStateView } from '../schemas.js';
import { type TurnState, type TurnStateRecord } from '../state.js';

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

const WAKE_ENQUEUE_ATTEMPTS = 3;
const WAKE_ENQUEUE_BACKOFF_MS = 250;

/**
 * Enqueue the next FSM step. Retried because a lost wake strands the record in
 * a non-terminal state with nothing scheduled to advance it (and run::start
 * then reports the session busy until its stale-takeover window). On final
 * failure we log at error level and rely on that takeover as the recovery
 * valve — re-enqueueing from a later save is suppressed by shouldWakeStep.
 */
async function enqueueTurnStep(iii: ISdk, session_id: string, state: TurnState): Promise<void> {
  for (let attempt = 1; attempt <= WAKE_ENQUEUE_ATTEMPTS; attempt++) {
    try {
      await iii.trigger({
        function_id: `turn::${state}`,
        payload: { session_id },
        action: TriggerAction.Enqueue({ queue: TURN_STEP_QUEUE }),
      });
      return;
    } catch (err) {
      if (attempt === WAKE_ENQUEUE_ATTEMPTS) {
        logger.error('wakeStep failed after retries; turn is stranded until stale takeover', {
          session_id,
          state,
          attempts: attempt,
          err: String(err),
        });
        return;
      }
      await new Promise((resolve) => setTimeout(resolve, WAKE_ENQUEUE_BACKOFF_MS * attempt));
    }
  }
}

export type AppendOptions = {
  /** Opaque correlation echoed on session events (e.g. `{ message_id }`). */
  origin?: Record<string, unknown>;
  /**
   * Deterministic entry id per message — the session-manager idempotency
   * key (§7 of integration.md). `function_result` messages default to
   * `fr-<function_call_id>` even without this hook.
   */
  entryIdFor?: (msg: AgentMessage, index: number) => string | undefined;
};

export type TurnStore = {
  loadRecord(session_id: string): Promise<TurnStateRecord | null>;
  /**
   * Like {@link loadRecord} but THROWS on a state-read failure instead of
   * tolerantly returning null. For guards that must fail closed: run::start's
   * in-flight check would clobber a live turn if a transient read error were
   * indistinguishable from "no record".
   */
  loadRecordStrict(session_id: string): Promise<TurnStateRecord | null>;
  saveRecord(rec: TurnStateRecord, previous?: TurnStateRecord | null): Promise<void>;
  writeRecord(rec: TurnStateRecord): Promise<void>;
  ensureSession(session_id: string): Promise<void>;
  appendMessages(session_id: string, msgs: AgentMessage[], opts?: AppendOptions): Promise<void>;
  loadRunRequest(session_id: string): Promise<RunRequest>;
  saveRunRequest(session_id: string, request: RunRequest): Promise<void>;
};

/**
 * Default deterministic id: function results carry a globally unique
 * function_call_id, so replayed batches dedupe without caller wiring.
 */
function defaultEntryId(msg: AgentMessage): string | undefined {
  if (msg.role === 'function_result') return functionResultEntryId(msg.function_call_id);
  return undefined;
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
  const result = await stateSet(iii, TURN_STATE_SCOPE, rec.session_id, rec);
  const prev = previous !== undefined ? previous : (result?.old_value ?? null);

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
  const strictState = createState(iii, { tolerant: false });

  return {
    async loadRecord(session_id) {
      // null = absent (no session); otherwise a record this version wrote.
      return stateGet<TurnStateRecord>(iii, TURN_STATE_SCOPE, session_id);
    },

    async loadRecordStrict(session_id) {
      // null = absent (no session); otherwise a record this version wrote.
      return strictState.get<TurnStateRecord>({ scope: TURN_STATE_SCOPE, key: session_id });
    },

    async writeRecord(rec) {
      await stateSet(iii, TURN_STATE_SCOPE, rec.session_id, rec);
    },

    async saveRecord(rec, previous) {
      const prev = await persistRecord(iii, rec, previous);
      if (shouldWakeStep(prev?.state ?? null, rec.state)) {
        await enqueueTurnStep(iii, rec.session_id, rec.state);
      }
    },

    /*
     * Idempotent, but invoked exactly once per run (at `run::start`) rather
     * than wrapping every read/write — the `run::start` gateway always
     * precedes any turn-store load/append for a session, so re-ensuring on
     * each call was pure RPC overhead.
     */
    async ensureSession(session_id) {
      await sessionEnsure(iii, session_id);
    },

    /*
     * Sequential appends without explicit parent_id: each append chains from
     * the active leaf and moves it, preserving order. Deterministic entry ids
     * make redelivered steps no-ops (existing entry returned, nothing fired).
     */
    async appendMessages(session_id, msgs, opts) {
      for (const [index, message] of msgs.entries()) {
        const entry_id = opts?.entryIdFor?.(message, index) ?? defaultEntryId(message);
        await sessionAppendMessage(iii, {
          session_id,
          message,
          ...(entry_id ? { entry_id } : {}),
          ...(opts?.origin ? { origin: opts.origin } : {}),
        });
      }
    },

    async saveRunRequest(session_id, request) {
      await stateSet(iii, RUN_REQUEST_SCOPE, session_id, request);
    },

    async loadRunRequest(session_id) {
      return (
        (await stateGet<RunRequest>(iii, RUN_REQUEST_SCOPE, session_id)) ?? defaultRunRequest()
      );
    },
  };
}
