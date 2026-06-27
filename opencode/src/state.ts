/**
 * Session registry on engine state. Scope `opencode_sessions`, key = iii
 * session_id. Maps iii sessions to OpenCode session ids so `opencode::run`
 * with the same session_id resumes the underlying OpenCode conversation.
 */

import type { IIIClient } from 'iii-sdk';
import type { SessionRecord } from './types.js';

const SCOPE = 'opencode_sessions';

export async function loadSession(
  iii: IIIClient,
  session_id: string,
): Promise<SessionRecord | null> {
  const res = await iii.trigger<unknown, SessionRecord | null>({
    function_id: 'state::get',
    payload: { scope: SCOPE, key: session_id },
  });
  return res && typeof res === 'object' && 'session_id' in res ? res : null;
}

export async function saveSession(iii: IIIClient, record: SessionRecord): Promise<void> {
  await iii.trigger({
    function_id: 'state::set',
    payload: { scope: SCOPE, key: record.session_id, value: record },
  });
}

export async function listSessions(iii: IIIClient): Promise<SessionRecord[]> {
  const res = await iii.trigger<unknown, SessionRecord[] | null>({
    function_id: 'state::list',
    payload: { scope: SCOPE },
  });
  return Array.isArray(res) ? res : [];
}
