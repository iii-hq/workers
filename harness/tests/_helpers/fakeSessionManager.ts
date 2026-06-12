/**
 * In-memory fake of the session-manager `session::*` surface — the subset
 * the harness drives: ensure, append (message/custom, idempotent on
 * entry_id), update_message, messages (path order, include_custom / roles
 * filters), set_status. Tests route `session::*` trigger calls into
 * `handle()` from their fake ISdk.
 */

import type { AgentMessage } from '../../src/types/agent-message.js';

export type FakeEntry = {
  entry_id: string;
  parent_id: string | null;
  kind: 'message' | 'custom';
  message?: AgentMessage;
  custom?: { custom_type: string; data: unknown };
  revision: number;
  origin?: Record<string, unknown>;
  timestamp: number;
};

export type FakeSession = {
  session_id: string;
  status: string;
  status_reason?: string;
  entries: FakeEntry[];
};

let autoId = 0;

export class FakeSessionManager {
  sessions = new Map<string, FakeSession>();
  calls: Array<{ function_id: string; payload: Record<string, unknown> }> = [];

  /** Route a trigger call. Returns undefined when not a session::* id. */
  async handle(function_id: string, payload: unknown): Promise<unknown> {
    if (!function_id.startsWith('session::')) return undefined;
    const p = (payload ?? {}) as Record<string, unknown>;
    this.calls.push({ function_id, payload: p });
    switch (function_id) {
      case 'session::ensure':
        return this.ensure(p);
      case 'session::append':
        return this.append(p);
      case 'session::update_message':
        return this.updateMessage(p);
      case 'session::messages':
        return this.messages(p);
      case 'session::set_status':
        return this.setStatus(p);
      default:
        throw new Error(`fake session-manager: unhandled ${function_id}`);
    }
  }

  callsTo(function_id: string): Array<Record<string, unknown>> {
    return this.calls.filter((c) => c.function_id === function_id).map((c) => c.payload);
  }

  session(session_id: string): FakeSession {
    let s = this.sessions.get(session_id);
    if (!s) {
      s = { session_id, status: 'idle', entries: [] };
      this.sessions.set(session_id, s);
    }
    return s;
  }

  /** Seed messages directly (path order), like a pre-existing transcript. */
  seedMessages(session_id: string, messages: AgentMessage[], entryIds?: string[]): string[] {
    const s = this.session(session_id);
    const ids: string[] = [];
    for (const [i, message] of messages.entries()) {
      const entry_id = entryIds?.[i] ?? `seed-${++autoId}`;
      s.entries.push({
        entry_id,
        parent_id: s.entries.at(-1)?.entry_id ?? null,
        kind: 'message',
        message,
        revision: 0,
        timestamp: Date.now(),
      });
      ids.push(entry_id);
    }
    return ids;
  }

  /** Seed a compaction custom entry at the current leaf. */
  seedCompaction(
    session_id: string,
    data: {
      summary: string;
      tokens_before?: number;
      tail_start_id?: string | null;
      timestamp?: number;
    },
  ): string {
    const s = this.session(session_id);
    const entry_id = `compaction-${++autoId}`;
    s.entries.push({
      entry_id,
      parent_id: s.entries.at(-1)?.entry_id ?? null,
      kind: 'custom',
      custom: {
        custom_type: 'compaction',
        data: {
          summary: data.summary,
          tokens_before: data.tokens_before ?? 0,
          tail_start_id: data.tail_start_id ?? null,
          details: {},
          timestamp: data.timestamp ?? Date.now(),
        },
      },
      revision: 0,
      timestamp: Date.now(),
    });
    return entry_id;
  }

  messageEntries(session_id: string): FakeEntry[] {
    return this.session(session_id).entries.filter((e) => e.kind === 'message');
  }

  private ensure(p: Record<string, unknown>) {
    const session_id = String(p.session_id);
    const created = !this.sessions.has(session_id);
    this.session(session_id);
    return { session_id, created };
  }

  private append(p: Record<string, unknown>) {
    const session_id = String(p.session_id);
    const s = this.session(session_id);
    const requestedId = typeof p.entry_id === 'string' ? p.entry_id : undefined;
    if (requestedId) {
      const existing = s.entries.find((e) => e.entry_id === requestedId);
      if (existing) {
        return {
          entry_id: existing.entry_id,
          parent_id: existing.parent_id,
          timestamp: existing.timestamp,
        };
      }
    }
    const entry_id = requestedId ?? `e-${++autoId}`;
    const parent_id =
      typeof p.parent_id === 'string' ? p.parent_id : (s.entries.at(-1)?.entry_id ?? null);
    const entry: FakeEntry = {
      entry_id,
      parent_id,
      kind: p.custom ? 'custom' : 'message',
      ...(p.custom
        ? { custom: p.custom as { custom_type: string; data: unknown } }
        : { message: p.message as AgentMessage }),
      revision: 0,
      ...(p.origin ? { origin: p.origin as Record<string, unknown> } : {}),
      timestamp: Date.now(),
    };
    s.entries.push(entry);
    return { entry_id, parent_id, timestamp: entry.timestamp };
  }

  private updateMessage(p: Record<string, unknown>) {
    const s = this.session(String(p.session_id));
    const entry = s.entries.find((e) => e.entry_id === p.entry_id);
    if (!entry) throw new Error(`session/entry_not_found: ${String(p.entry_id)}`);
    if (entry.kind !== 'message' || !entry.message) {
      throw new Error('session/invalid_entry_kind: custom entry');
    }
    entry.message = {
      ...entry.message,
      content: p.content as AgentMessage['content'],
    } as AgentMessage;
    if (p.details !== undefined && 'details' in entry.message) {
      (entry.message as { details?: unknown }).details = p.details;
    }
    entry.revision += 1;
    return { updated: true, revision: entry.revision };
  }

  private messages(p: Record<string, unknown>) {
    const s = this.session(String(p.session_id));
    const roles = Array.isArray(p.roles) ? (p.roles as string[]) : null;
    const include_custom = p.include_custom === true;
    const out: Array<{
      entry_id: string;
      message?: AgentMessage;
      custom?: { custom_type: string; data: unknown };
    }> = [];
    for (const e of s.entries) {
      if (e.kind === 'message' && e.message) {
        if (roles && !roles.includes(e.message.role)) continue;
        out.push({ entry_id: e.entry_id, message: e.message });
      } else if (e.kind === 'custom' && e.custom) {
        if (roles) continue;
        if (!include_custom) continue;
        out.push({ entry_id: e.entry_id, custom: e.custom });
      }
    }
    // Single page: tests stay below the 500-row cap.
    return { messages: out };
  }

  private setStatus(p: Record<string, unknown>) {
    const s = this.session(String(p.session_id));
    const previous_status = s.status;
    s.status = String(p.status);
    s.status_reason = p.status === 'error' && typeof p.reason === 'string' ? p.reason : undefined;
    return { status: s.status, previous_status };
  }
}
