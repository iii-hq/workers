/**
 * State load/save helpers. All `state::*` I/O goes through
 * `../runtime/state.js` (agent scope).
 */

import { stateGet, stateListValues, stateSet } from '../runtime/state.js';
import type { ISdk } from '../runtime/iii.js';
import type { AgentMessage } from '../types/agent-message.js';
import { parseFlatMessages } from './flat-messages.js';
import { type RunRequest, parseRunRequest } from './run-request.js';
import {
  AGENT_SCOPE,
  SESSION_INDEX_SCOPE,
  type TurnStateRecord,
  messagesKey,
  parseTurnStateRecord,
  runRequestKey,
  turnStateKey,
} from './state.js';
import { toView } from './schemas.js';
import { mirrorMessagesToSessionTree } from './session-tree-mirror.js';
import { emitTurnStateChanged } from './turn-state-write.js';
import { shouldWakeStep, wakeState } from './wake.js';

const agentGet = (iii: ISdk, key: string) => stateGet(iii, AGENT_SCOPE, key);
const agentSet = (iii: ISdk, key: string, value: unknown) =>
  stateSet(iii, AGENT_SCOPE, key, value);

// --- turn_state ---

export async function loadRecord(iii: ISdk, session_id: string): Promise<TurnStateRecord | null> {
  return parseTurnStateRecord(await agentGet(iii, turnStateKey(session_id)));
}

/** All turn_state values in agent scope; non-records are dropped by shape parse. */
export async function listAgentTurnStateRecords(iii: ISdk): Promise<TurnStateRecord[]> {
  const values = await stateListValues<unknown>(iii, { scope: AGENT_SCOPE });
  return values
    .map((value) => parseTurnStateRecord(value))
    .filter((rec): rec is TurnStateRecord => rec !== null);
}

/** Silent checkpoint — no UI event, no FSM wake. */
export async function writeRecord(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  await agentSet(iii, turnStateKey(rec.session_id), rec);
}

async function persistRecord(
  iii: ISdk,
  rec: TurnStateRecord,
  previous?: TurnStateRecord | null,
): Promise<TurnStateRecord | null> {
  const result = await agentSet(iii, turnStateKey(rec.session_id), rec);
  const prev =
    previous !== undefined ? previous : parseTurnStateRecord(result?.old_value ?? null);

  if (prev == null) {
    // First persist for this session → mark it in the session index. The
    // create-fanout trigger watches that dedicated scope, so it matches in
    // engine by scope alone — no per-write condition predicate.
    await stateSet(iii, SESSION_INDEX_SCOPE, rec.session_id, { created_at_ms: Date.now() });
  }

  await emitTurnStateChanged(
    iii,
    rec.session_id,
    prev == null ? 'state:created' : 'state:updated',
    toView(rec),
    prev != null ? toView(prev) : undefined,
  );

  return prev;
}

export async function saveRecord(
  iii: ISdk,
  rec: TurnStateRecord,
  previous?: TurnStateRecord | null,
): Promise<void> {
  const prev = await persistRecord(iii, rec, previous);

  if (shouldWakeStep(prev?.state ?? null, rec.state)) {
    await wakeState(iii, rec.session_id, rec.state);
  }
}

// --- messages ---

export async function loadMessages(iii: ISdk, session_id: string): Promise<AgentMessage[]> {
  return parseFlatMessages(await agentGet(iii, messagesKey(session_id)));
}

export async function saveMessages(
  iii: ISdk,
  session_id: string,
  messages: AgentMessage[],
): Promise<void> {
  await agentSet(iii, messagesKey(session_id), messages);
  await mirrorMessagesToSessionTree(iii, session_id, messages);
}

// --- run_request ---

export async function saveRunRequest(
  iii: ISdk,
  session_id: string,
  request: RunRequest,
): Promise<void> {
  await agentSet(iii, runRequestKey(session_id), request);
}

export async function loadRunRequest(iii: ISdk, session_id: string): Promise<RunRequest> {
  return parseRunRequest(await agentGet(iii, runRequestKey(session_id)));
}
