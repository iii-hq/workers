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
  /**
   * Append several pre-chained entries, refreshing session meta once for the
   * whole batch (vs once per {@link append}). Entries are written in order.
   */
  appendMany(session_id: string, entries: SessionEntry[]): Promise<void>;
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

  async appendMany(session_id: string, entries: SessionEntry[]): Promise<void> {
    const list = this.entries.get(session_id);
    if (!list) throw new SessionError('not_found', `session not found: ${session_id}`);
    list.push(...entries);
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

  /** Write one entry under its own key. Throws on storage failure. */
  private async writeEntry(session_id: string, key: string, entry: SessionEntry): Promise<void> {
    try {
      await this.state.set({ scope: entriesScope(session_id), key, value: entry });
    } catch (e) {
      throw new SessionError('storage', `state::set entry: ${String(e)}`);
    }
  }

  /** Bump meta.updated_at; best-effort, failures log only. */
  private async bestEffortMetaRefresh(session_id: string): Promise<void> {
    try {
      await this.refreshMetaUpdatedAt(session_id);
    } catch (e) {
      logger.warn('meta updated_at refresh failed', { session_id, err: String(e) });
    }
  }

  async append(session_id: string, entry: SessionEntry): Promise<void> {
    await this.writeEntry(session_id, entry.id, entry);
    await this.bestEffortMetaRefresh(session_id);
  }

  async appendMany(session_id: string, entries: SessionEntry[]): Promise<void> {
    for (const entry of entries) {
      await this.writeEntry(session_id, entry.id, entry);
    }
    await this.bestEffortMetaRefresh(session_id);
  }

  async loadEntries(session_id: string): Promise<SessionEntry[]> {
    let entries: SessionEntry[];
    try {
      entries = await this.state.list<SessionEntry>({ scope: entriesScope(session_id) });
    } catch (e) {
      throw new SessionError('storage', `state::list entries: ${String(e)}`);
    }
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
    await this.writeEntry(session_id, entry_id, updated);
    await this.bestEffortMetaRefresh(session_id);
  }

  private async refreshMetaUpdatedAt(session_id: string): Promise<void> {
    const value = await this.state.get<SessionMeta>({ scope: META_SCOPE, key: session_id });
    if (value === null) return;
    const meta = { ...value, updated_at: Date.now() };
    await this.state.set({ scope: META_SCOPE, key: session_id, value: meta });
  }
}
