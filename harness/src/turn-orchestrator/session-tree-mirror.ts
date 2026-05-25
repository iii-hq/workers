/**
 * Incrementally mirrors flat agent messages into the session-tree store.
 */

import { z } from 'zod';
import { stateGet, stateSet } from '../runtime/state.js';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AgentMessage } from '../types/agent-message.js';

const SESSION_TREE_MIRROR_LEN_SCOPE = 'session_tree_mirror_len';

const MirrorLenSchema = z.coerce.number().int().nonnegative().catch(0);

export function parseMirrorLen(raw: unknown): number {
  return MirrorLenSchema.parse(raw ?? 0);
}

export async function mirrorMessagesToSessionTree(
  iii: ISdk,
  session_id: string,
  messages: AgentMessage[],
): Promise<void> {
  const alreadyMirrored = parseMirrorLen(
    await stateGet(iii, SESSION_TREE_MIRROR_LEN_SCOPE, session_id),
  );
  if (messages.length <= alreadyMirrored) return;

  if (alreadyMirrored === 0) {
    const ensured = await triggerSessionTree(iii, 'session-tree::ensure', { session_id });
    if (!ensured) return;
  }

  let lastAppended: string | null = null;
  if (alreadyMirrored > 0) {
    const resp = await triggerSessionTree<{ messages?: Array<{ entry_id?: string }> }>(
      iii,
      'session-tree::messages',
      { session_id },
    );
    if (!resp) return;
    const tail = resp.messages?.at(-1);
    lastAppended = tail?.entry_id ?? null;
  }

  for (const msg of messages.slice(alreadyMirrored)) {
    const resp = await triggerSessionTree<{ entry_id?: string }>(
      iii,
      'session-tree::append',
      { session_id, parent_id: lastAppended, message: msg },
    );
    if (!resp) return;
    lastAppended = resp.entry_id ?? lastAppended;
  }

  await stateSet(iii, SESSION_TREE_MIRROR_LEN_SCOPE, session_id, messages.length);
}

async function triggerSessionTree<T>(
  iii: ISdk,
  function_id: string,
  payload: Record<string, unknown>,
): Promise<T | null> {
  try {
    return await iii.trigger<unknown, T>({ function_id, payload });
  } catch (err) {
    logger.warn(`${function_id} failed; session-tree mirror skipped`, {
      session_id: payload.session_id,
      err: String(err),
    });
    return null;
  }
}
