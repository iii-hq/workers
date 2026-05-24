/**
 * State load/save helpers. Mirrors `turn-orchestrator/src/persistence.rs`.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AgentMessage } from '../types/agent-message.js';
import type { FunctionCall, FunctionResult } from '../types/function.js';
import { type RunRequest, parseRunRequest } from './run-request.js';
import {
  type ExecutedEntry,
  type PreparedEntry,
  type TurnStateRecord,
  functionSchemasKey,
  lastSessionTreeLenKey,
  messagesKey,
  runRequestKey,
  turnStateKey,
} from './state.js';
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

export async function loadRecord(iii: ISdk, session_id: string): Promise<TurnStateRecord | null> {
  const v = await stateGet(iii, turnStateKey(session_id));
  if (!v || typeof v !== 'object') return null;
  return v as TurnStateRecord;
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
    rec as unknown as Record<string, unknown>,
    prev !== null ? (prev as unknown as Record<string, unknown>) : undefined,
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

export async function saveFunctionSchemas(
  iii: ISdk,
  session_id: string,
  schemas: unknown,
): Promise<void> {
  await stateSet(iii, functionSchemasKey(session_id), schemas);
}

export async function loadFunctionSchemas(iii: ISdk, session_id: string): Promise<unknown[]> {
  const v = await stateGet(iii, functionSchemasKey(session_id));
  return Array.isArray(v) ? v : [];
}

const PREPARED_KEY = 'function_prepared';
const EXECUTED_KEY = 'function_executed';

const stagingKey = (sid: string, suffix: string) => `session/${sid}/${suffix}`;

async function stagingGet(iii: ISdk, session_id: string, suffix: string): Promise<unknown[]> {
  const v = await stateGet(iii, stagingKey(session_id, suffix));
  return Array.isArray(v) ? v : [];
}


export async function savePreparedCalls(
  iii: ISdk,
  session_id: string,
  prepared: PreparedEntry[],
): Promise<void> {
  const payload = prepared.map((e) => ({
    function_call: e.function_call,
    blocked: e.blocked,
    pre_approved: e.pre_approved ?? false,
  }));
  await stateSet(iii, stagingKey(session_id, PREPARED_KEY), payload);
}

export async function loadPreparedCalls(iii: ISdk, session_id: string): Promise<PreparedEntry[]> {
  const items = await stagingGet(iii, session_id, PREPARED_KEY);
  const out: PreparedEntry[] = [];
  for (const it of items) {
    if (!it || typeof it !== 'object') continue;
    const obj = it as Record<string, unknown>;
    const fc = obj.function_call as FunctionCall | undefined;
    if (!fc) continue;
    const blocked = (obj.blocked as FunctionResult | null) ?? null;
    const pre_approved = obj.pre_approved === true;
    out.push({ function_call: fc, blocked, pre_approved });
  }
  return out;
}

export async function saveExecutedCalls(
  iii: ISdk,
  session_id: string,
  executed: ExecutedEntry[],
): Promise<void> {
  await stateSet(iii, stagingKey(session_id, EXECUTED_KEY), executed);
}

export async function loadExecutedCalls(iii: ISdk, session_id: string): Promise<ExecutedEntry[]> {
  const items = await stagingGet(iii, session_id, EXECUTED_KEY);
  const out: ExecutedEntry[] = [];
  for (const it of items) {
    if (!it || typeof it !== 'object') continue;
    const obj = it as Record<string, unknown>;
    const fc = obj.function_call as FunctionCall | undefined;
    const result = obj.result as FunctionResult | undefined;
    if (!fc || !result) continue;
    out.push({
      function_call: fc,
      result,
      is_error: typeof obj.is_error === 'boolean' ? obj.is_error : false,
      duration_ms: typeof obj.duration_ms === 'number' ? obj.duration_ms : 0,
    });
  }
  return out;
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
