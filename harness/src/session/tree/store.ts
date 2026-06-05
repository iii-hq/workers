/**
 * Session storage backend.
 *
 * - `InMemoryStore` is used by unit tests only (constructed directly, not via worker config).
 * - `IiiStateSessionStore` mirrors
 *   `session/src/tree/store_iii_state.rs`. Storage layout:
 *
 *     scope `session_tree:<session_id>`, key `<entry_id>` → SessionEntry
 *     scope `session_tree_meta`, key `<session_id>`     → SessionMeta
 *
 * `state::list` returns values without keys, so we re-sort entries by
 * `(timestamp, id)` after reading (matches the Rust crate, post PR #150).
 */

import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { createState } from '../../runtime/state.js';
import { type SessionEntry, SessionError, type SessionMeta, entryTimestamp } from './types.js';

export interface SessionStore {
  create(meta: SessionMeta): Promise<void>;
  append(session_id: string, entry: SessionEntry): Promise<void>;
  loadEntries(session_id: string): Promise<SessionEntry[]>;
  loadMeta(session_id: string): Promise<SessionMeta>;
  list(): Promise<SessionMeta[]>;
  /** Replace the entry with the given id in-place. No-op if id not found. */
  updateEntry(session_id: string, entry_id: string, updated: SessionEntry): Promise<void>;
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

  async updateEntry(session_id: string, entry_id: string, updated: SessionEntry): Promise<void> {
    const list = this.entries.get(session_id);
    if (!list) return;
    const idx = list.findIndex((e) => e.id === entry_id);
    if (idx === -1) return;
    list[idx] = updated;
    const m = this.meta.get(session_id);
    if (m) m.updated_at = Date.now();
  }
}

const META_SCOPE = 'session_tree_meta';

function entriesScope(session_id: string): string {
  return `session_tree:${session_id}`;
}

export class IiiStateSessionStore implements SessionStore {
  private readonly state;

  constructor(iii: ISdk) {
    this.state = createState(iii, { tolerant: false });
  }

  async create(meta: SessionMeta): Promise<void> {
    try {
      await this.state.set({ scope: META_SCOPE, key: meta.session_id, value: meta });
    } catch (e) {
      throw new SessionError('storage', `state::set meta: ${String(e)}`);
    }
  }

  async append(session_id: string, entry: SessionEntry): Promise<void> {
    try {
      await this.state.set({
        scope: entriesScope(session_id),
        key: entry.id,
        value: entry,
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
    let entries: SessionEntry[];
    try {
      entries = await this.state.list<SessionEntry>({ scope: entriesScope(session_id) });
    } catch (e) {
      throw new SessionError('storage', `state::list entries: ${String(e)}`);
    }
    // PR #150: sort by (timestamp, id) so resumed approval replies that
    // arrive after the session paused appear in correct transcript order
    // even when their entry ids are non-monotonic.
    entries.sort((a, b) => {
      const t = entryTimestamp(a) - entryTimestamp(b);
      if (t !== 0) return t;
      return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
    });
    return entries;
  }

  async loadMeta(session_id: string): Promise<SessionMeta> {
    let resp: SessionMeta | null;
    try {
      resp = await this.state.get<SessionMeta>({ scope: META_SCOPE, key: session_id });
    } catch (e) {
      throw new SessionError('storage', `state::get meta: ${String(e)}`);
    }
    if (resp === null) {
      throw new SessionError('not_found', session_id);
    }
    return resp;
  }

  async list(): Promise<SessionMeta[]> {
    try {
      return await this.state.list<SessionMeta>({ scope: META_SCOPE });
    } catch (e) {
      throw new SessionError('storage', `state::list meta: ${String(e)}`);
    }
  }

  async updateEntry(session_id: string, entry_id: string, updated: SessionEntry): Promise<void> {
    try {
      await this.state.set({
        scope: entriesScope(session_id),
        key: entry_id,
        value: updated,
      });
    } catch (e) {
      throw new SessionError('storage', `state::set updateEntry: ${String(e)}`);
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

  private async refreshMetaUpdatedAt(session_id: string): Promise<void> {
    const value = await this.state.get<SessionMeta>({ scope: META_SCOPE, key: session_id });
    if (value === null) return;
    const meta = { ...value, updated_at: Date.now() };
    await this.state.set({ scope: META_SCOPE, key: session_id, value: meta });
  }
}
