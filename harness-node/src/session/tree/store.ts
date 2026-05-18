/**
 * Session storage backend.
 *
 * - `InMemoryStore` is the test/replay backend.
 * - `IiiStateSessionStore` mirrors
 *   `session/src/tree/store_iii_state.rs`. Storage layout:
 *
 *     scope `session_tree:<session_id>`, key `<entry_id>` → SessionEntry
 *     scope `session_tree_meta`, key `<session_id>`     → SessionMeta
 *
 * `state::list` returns values without keys, so we re-sort entries by
 * `id` after reading (matches the Rust crate).
 */

import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { type SessionEntry, SessionError, type SessionMeta } from './types.js';

export interface SessionStore {
  create(meta: SessionMeta): Promise<void>;
  append(session_id: string, entry: SessionEntry): Promise<void>;
  loadEntries(session_id: string): Promise<SessionEntry[]>;
  loadMeta(session_id: string): Promise<SessionMeta>;
  list(): Promise<SessionMeta[]>;
}

export class InMemoryStore implements SessionStore {
  private readonly entries = new Map<string, SessionEntry[]>();
  private readonly meta = new Map<string, SessionMeta>();

  async create(meta: SessionMeta): Promise<void> {
    this.meta.set(meta.session_id, meta);
    if (!this.entries.has(meta.session_id)) this.entries.set(meta.session_id, []);
  }

  async append(session_id: string, entry: SessionEntry): Promise<void> {
    const list = this.entries.get(session_id);
    if (!list) throw new SessionError('not_found', `session not found: ${session_id}`);
    list.push(entry);
    const m = this.meta.get(session_id);
    if (m) m.updated_at = Date.now();
  }

  async loadEntries(session_id: string): Promise<SessionEntry[]> {
    return [...(this.entries.get(session_id) ?? [])];
  }

  async loadMeta(session_id: string): Promise<SessionMeta> {
    const m = this.meta.get(session_id);
    if (!m) throw new SessionError('not_found', `session not found: ${session_id}`);
    return { ...m };
  }

  async list(): Promise<SessionMeta[]> {
    return [...this.meta.values()].map((m) => ({ ...m }));
  }
}

const META_SCOPE = 'session_tree_meta';

function entriesScope(session_id: string): string {
  return `session_tree:${session_id}`;
}

export class IiiStateSessionStore implements SessionStore {
  constructor(private readonly iii: ISdk) {}

  async create(meta: SessionMeta): Promise<void> {
    try {
      await this.iii.trigger<unknown, unknown>({
        function_id: 'state::set',
        payload: { scope: META_SCOPE, key: meta.session_id, value: meta },
      });
    } catch (e) {
      throw new SessionError('storage', `state::set meta: ${String(e)}`);
    }
  }

  async append(session_id: string, entry: SessionEntry): Promise<void> {
    try {
      await this.iii.trigger<unknown, unknown>({
        function_id: 'state::set',
        payload: {
          scope: entriesScope(session_id),
          key: entry.id,
          value: entry,
        },
      });
    } catch (e) {
      throw new SessionError('storage', `state::set entry: ${String(e)}`);
    }
    // Best-effort meta refresh — failures here log only.
    try {
      await this.refreshMetaUpdatedAt(session_id);
    } catch (e) {
      logger.warn('meta updated_at refresh failed', {
        session_id,
        err: String(e),
      });
    }
  }

  async loadEntries(session_id: string): Promise<SessionEntry[]> {
    let resp: unknown;
    try {
      resp = await this.iii.trigger<unknown, unknown>({
        function_id: 'state::list',
        payload: { scope: entriesScope(session_id) },
      });
    } catch (e) {
      throw new SessionError('storage', `state::list entries: ${String(e)}`);
    }
    const arr = pickArray(resp);
    if (!arr) throw new SessionError('storage', 'state::list returned non-array');
    const entries = arr.map((v) => unwrapValue<SessionEntry>(v));
    entries.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
    return entries;
  }

  async loadMeta(session_id: string): Promise<SessionMeta> {
    let resp: unknown;
    try {
      resp = await this.iii.trigger<unknown, unknown>({
        function_id: 'state::get',
        payload: { scope: META_SCOPE, key: session_id },
      });
    } catch (e) {
      throw new SessionError('storage', `state::get meta: ${String(e)}`);
    }
    if (resp === null || resp === undefined) {
      throw new SessionError('not_found', session_id);
    }
    return resp as SessionMeta;
  }

  async list(): Promise<SessionMeta[]> {
    let resp: unknown;
    try {
      resp = await this.iii.trigger<unknown, unknown>({
        function_id: 'state::list',
        payload: { scope: META_SCOPE },
      });
    } catch (e) {
      throw new SessionError('storage', `state::list meta: ${String(e)}`);
    }
    const arr = pickArray(resp);
    if (!arr) throw new SessionError('storage', 'state::list returned non-array');
    return arr.map((v) => unwrapValue<SessionMeta>(v));
  }

  private async refreshMetaUpdatedAt(session_id: string): Promise<void> {
    const value = await this.iii.trigger<unknown, unknown>({
      function_id: 'state::get',
      payload: { scope: META_SCOPE, key: session_id },
    });
    if (value === null || value === undefined) return;
    const meta = value as SessionMeta;
    meta.updated_at = Date.now();
    await this.iii.trigger<unknown, unknown>({
      function_id: 'state::set',
      payload: { scope: META_SCOPE, key: session_id, value: meta },
    });
  }
}

function pickArray(v: unknown): unknown[] | null {
  if (Array.isArray(v)) return v;
  if (v && typeof v === 'object') {
    const items = (v as Record<string, unknown>).items;
    if (Array.isArray(items)) return items;
  }
  return null;
}

function unwrapValue<T>(v: unknown): T {
  if (v && typeof v === 'object' && 'value' in (v as Record<string, unknown>)) {
    return (v as Record<string, unknown>).value as T;
  }
  return v as T;
}
