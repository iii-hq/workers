/**
 * Session registry on engine state. Scope `pi_sessions`, key = iii session_id.
 * Maps iii sessions to Pi session files so `pi::run` with the same session_id
 * resumes the underlying Pi conversation.
 */

import type { ISdk } from 'iii-sdk';
import type { SessionRecord } from './types.js';

const SCOPE = 'pi_sessions';

export async function loadSession(iii: ISdk, session_id: string): Promise<SessionRecord | null> {
  const res = await iii.trigger<unknown, SessionRecord | null>({
    function_id: 'state::get',
    payload: { scope: SCOPE, key: session_id },
  });
  return res && typeof res === 'object' && 'session_id' in res ? res : null;
}

export async function saveSession(iii: ISdk, record: SessionRecord): Promise<void> {
  await iii.trigger({
    function_id: 'state::set',
    payload: { scope: SCOPE, key: record.session_id, value: record },
  });
}

export async function listSessions(iii: ISdk): Promise<SessionRecord[]> {
  const res = await iii.trigger<unknown, SessionRecord[] | null>({
    function_id: 'state::list',
    payload: { scope: SCOPE },
  });
  return Array.isArray(res) ? res : [];
}
