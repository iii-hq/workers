/**
 * `session-inbox::push|peek|drain` handlers. Mirrors
 * `session/src/inbox/handler.rs`.
 */

import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { stateGet, stateUpdate } from '../../runtime/state.js';
import { inboxKey } from './key.js';

export const PUSH_ID = 'session-inbox::push';
export const DRAIN_ID = 'session-inbox::drain';
export const PEEK_ID = 'session-inbox::peek';

export type InboxConfig = { state_scope: string };

export function registerInbox(iii: ISdk, cfg: InboxConfig): void {
  iii.registerFunction(
    PUSH_ID,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const name = requireString(obj, 'name');
      const session_id = requireString(obj, 'session_id');
      if (!('item' in obj)) throw new Error('missing required field: item');
      await stateUpdate(iii, cfg.state_scope, inboxKey(name, session_id), [
        { type: 'append', value: obj.item, path: '' },
      ]);
      return { ok: true };
    },
    { description: 'Append an item to a session-scoped inbox.' },
  );

  iii.registerFunction(
    PEEK_ID,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const name = requireString(obj, 'name');
      const session_id = requireString(obj, 'session_id');
      const value = await stateGet(iii, cfg.state_scope, inboxKey(name, session_id));
      const items = value === null ? [] : value;
      return { items };
    },
    { description: 'Read all items in a session-scoped inbox without mutating.' },
  );

  iii.registerFunction(
    DRAIN_ID,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const name = requireString(obj, 'name');
      const session_id = requireString(obj, 'session_id');
      const resp = await stateUpdate(iii, cfg.state_scope, inboxKey(name, session_id), [
        { type: 'set', value: [], path: '' },
      ]);
      if (!resp) {
        logger.warn('inbox drain: state::update failed; returning empty', {
          name,
          session_id,
        });
        return { items: [] };
      }
      return { items: resp.old_value ?? [] };
    },
    { description: 'Atomically read and clear all items in a session-scoped inbox.' },
  );
}
