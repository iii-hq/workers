/**
 * Session tree wire types. Mirror
 * `session/src/tree/mod.rs::{SessionEntry, SessionMeta, TreeNode, …}`.
 */

import type { AgentMessage } from '../../types/agent-message.js';

export type CompactionDetails = {
  read_files?: string[];
  modified_files?: string[];
};

export type SessionEntry =
  | {
      type: 'message';
      id: string;
      parent_id?: string | null;
      message: AgentMessage;
      timestamp: number;
    }
  | {
      type: 'custom_message';
      id: string;
      parent_id?: string | null;
      custom_type: string;
      content: unknown;
      display?: string;
      details?: unknown;
      timestamp: number;
    }
  | {
      type: 'branch_summary';
      id: string;
      parent_id?: string | null;
      summary: string;
      from_id: string;
      timestamp: number;
    }
  | {
      type: 'compaction';
      id: string;
      parent_id?: string | null;
      summary: string;
      tokens_before: number;
      details: CompactionDetails;
      timestamp: number;
    };

export function entryId(e: SessionEntry): string {
  return e.id;
}

export function entryParent(e: SessionEntry): string | null {
  return e.parent_id ?? null;
}

/**
 * Returns the entry's wall-clock timestamp. Mirrors
 * `session/src/tree/mod.rs::SessionEntry::timestamp` (PR #150).
 *
 * Used by `IiiStateSessionStore.loadEntries` to sort entries by
 * `(timestamp, id)` so resumed approval replies that arrive with newer
 * timestamps but older ids land in the correct transcript position.
 */
export function entryTimestamp(e: SessionEntry): number {
  return e.timestamp;
}

export type SessionMeta = {
  session_id: string;
  display_name?: string;
  created_at: number;
  updated_at: number;
  cwd?: string;
  branch_count: number;
};

export type TreeNode = {
  entry: SessionEntry;
  children: TreeNode[];
};

export type MessageWithEntryId = {
  entry_id: string;
  message: AgentMessage;
};

export type ReconcileResult = {
  state_count: number;
  tree_count_before: number;
  tree_count_after: number;
  repaired: number;
};

export type ListOrder = 'asc' | 'desc';

export type SessionListRow = {
  session_id: string;
  created_at: number;
  updated_at: number;
  entry_count: number;
  display_name?: string;
  cwd?: string;
  last_message_summary?: string;
};

export type ListSessionsResult = {
  sessions: SessionListRow[];
  total: number;
};

export class SessionError extends Error {
  readonly kind: 'not_found' | 'entry_not_found' | 'storage';
  constructor(kind: 'not_found' | 'entry_not_found' | 'storage', message: string) {
    super(message);
    this.kind = kind;
    this.name = 'SessionError';
  }
}
