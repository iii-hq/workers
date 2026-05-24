/**
 * State load/save helpers. Mirrors `turn-orchestrator/src/persistence.rs`.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AgentMessage } from '../types/agent-message.js';
import { type RunRequest, parseRunRequest } from './run-request.js';
import {
  type ExecutedEntry,
  type PreparedEntry,
  type TurnStateRecord,
  lastSessionTreeLenKey,
  messagesKey,
  runRequestKey,
  turnStateKey,
} from './state.js';
import { toView } from './schemas.js';
import { emitTurnStateChanged } from './turn-state-write.js';
import { shouldWakeStep, wakeState } from './wake.js';

export type { ExecutedEntry, PreparedEntry } from './state.js';

const SCOPE = 'agent';

async function stateGet(iii: ISdk, key: string): Promise<unknown | null> {
  try {
    const v = await iii.trigger<unknown, unknown>({
      function_id: 'state::get',
      payload: { scope: SCOPE, key },
    });
    return v === null || v === undefined ? null : v;
  } catch (err) {
    logger.warn('persistence state::get failed', { key, err: String(err) });
    return null;
  }
}

async function stateSet(iii: ISdk, key: string, value: unknown): Promise<void> {
  try {
    await iii.trigger<unknown, unknown>({
      function_id: 'state::set',
      payload: { scope: SCOPE, key, value },
    });
  } catch (err) {
    logger.warn('persistence state::set failed', { key, err: String(err) });
  }
}

/** Defensive coercion for records persisted before assistant_finished was
 *  removed. Drain-before-cutover is preferred; this prevents a crash on an
 *  in-flight legacy record. */
export function migrateLegacyRecord(rec: TurnStateRecord): TurnStateRecord {
  if ((rec.state as string) === 'assistant_finished') {
    const asst = rec.last_assistant;
    const hasCalls = !!asst && asst.content.some((b) => b.type === 'function_call');
    return { ...rec, state: hasCalls ? 'function_execute' : 'steering_check' };
  }
  return rec;
}

export async function loadRecord(iii: ISdk, session_id: string): Promise<TurnStateRecord | null> {
  const v = await stateGet(iii, turnStateKey(session_id));
  if (!v || typeof v !== 'object') return null;
  return migrateLegacyRecord(v as TurnStateRecord);
}

/** Persist the record with no UI event and no FSM wake — for mid-handler,
 *  same-state checkpoints (e.g. per function-call result during execute). */
export async function writeRecord(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  await stateSet(iii, turnStateKey(rec.session_id), rec);
}

/**
 * Persist turn_state and emit UI event — no FSM wake (mid-handler saves).
 * Pass `previous` (the pre-write record) to skip the `state::get` that would
 * otherwise re-read it; omit it and the prior value is loaded here.
 */
export async function persistRecord(
  iii: ISdk,
  rec: TurnStateRecord,
  previous?: TurnStateRecord | null,
): Promise<void> {
  const prev = previous !== undefined ? previous : await loadRecord(iii, rec.session_id);
  const eventType = prev === null ? 'state:created' : 'state:updated';

  await stateSet(iii, turnStateKey(rec.session_id), rec);

  await emitTurnStateChanged(
    iii,
    rec.session_id,
    eventType,
    toView(rec) as unknown as Record<string, unknown>,
    prev !== null ? (toView(prev) as unknown as Record<string, unknown>) : undefined,
  );
}

export async function saveRecord(
  iii: ISdk,
  rec: TurnStateRecord,
  previous?: TurnStateRecord | null,
): Promise<void> {
  const prev = previous !== undefined ? previous : await loadRecord(iii, rec.session_id);
  await persistRecord(iii, rec, prev);

  if (shouldWakeStep(prev?.state ?? null, rec.state)) {
    await wakeState(iii, rec.session_id, rec.state);
  }
}

export async function loadMessages(iii: ISdk, session_id: string): Promise<AgentMessage[]> {
  const v = await stateGet(iii, messagesKey(session_id));
  return Array.isArray(v) ? (v as AgentMessage[]) : [];
}

export async function saveMessages(
  iii: ISdk,
  session_id: string,
  messages: AgentMessage[],
): Promise<void> {
  await stateSet(iii, messagesKey(session_id), messages);
  await mirrorMessagesToSessionTree(iii, session_id, messages);
}

async function mirrorMessagesToSessionTree(
  iii: ISdk,
  session_id: string,
  messages: AgentMessage[],
): Promise<void> {
  const lastKey = lastSessionTreeLenKey(session_id);
  const last = await stateGet(iii, lastKey);
  const alreadyMirrored = typeof last === 'number' ? last : 0;
  if (messages.length <= alreadyMirrored) return;
  if (alreadyMirrored === 0) {
    try {
      await iii.trigger<unknown, unknown>({
        function_id: 'session-tree::ensure',
        payload: { session_id },
      });
    } catch (err) {
      logger.warn('session-tree::ensure failed; mirror skipped', {
        session_id,
        err: String(err),
      });
      return;
    }
  }
  let lastAppended: string | null = null;
  if (alreadyMirrored > 0) {
    try {
      const resp = await iii.trigger<unknown, { messages?: Array<{ entry_id?: string }> }>({
        function_id: 'session-tree::messages',
        payload: { session_id },
      });
      const arr = resp?.messages;
      if (Array.isArray(arr) && arr.length > 0) {
        const tail = arr[arr.length - 1];
        lastAppended = tail?.entry_id ?? null;
      }
    } catch (err) {
      logger.warn('session-tree::messages read failed mid-mirror; skipping', {
        session_id,
        err: String(err),
      });
      return;
    }
  }
  for (const msg of messages.slice(alreadyMirrored)) {
    try {
      const resp = await iii.trigger<unknown, { entry_id?: string }>({
        function_id: 'session-tree::append',
        payload: { session_id, parent_id: lastAppended, message: msg },
      });
      lastAppended = resp?.entry_id ?? lastAppended;
    } catch (err) {
      logger.warn('session-tree::append mirror failed', { session_id, err: String(err) });
      return;
    }
  }
  await stateSet(iii, lastKey, messages.length);
}

export async function saveRunRequest(
  iii: ISdk,
  session_id: string,
  request: unknown,
): Promise<void> {
  await stateSet(iii, runRequestKey(session_id), request);
}

export async function loadRunRequest(iii: ISdk, session_id: string): Promise<RunRequest> {
  const v = await stateGet(iii, runRequestKey(session_id));
  return parseRunRequest(v && typeof v === 'object' ? (v as Record<string, unknown>) : {});
}

export function findExecutedCall(
  executed: ExecutedEntry[],
  function_call_id: string,
): ExecutedEntry | undefined {
  return executed.find((e) => e.function_call.id === function_call_id);
}

export function upsertExecutedCall(executed: ExecutedEntry[], entry: ExecutedEntry): void {
  const idx = executed.findIndex((e) => e.function_call.id === entry.function_call.id);
  if (idx >= 0) executed[idx] = entry;
  else executed.push(entry);
}
