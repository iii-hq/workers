/**
 * Register all 11 `session-tree::*` functions. Mirrors
 * `session/src/tree/mod.rs::register_with_iii`.
 */

import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import type { AgentMessage } from '../../types/agent-message.js';
import {
  activePath as _activePath,
  appendMessage,
  cloneSession,
  compact as compactOp,
  createSession,
  ensureSession,
  exportHtml,
  fork,
  listSessions,
  loadMessagesWithEntryIds,
  reconcile,
  tree,
} from './operations.js';
import type { SessionStore } from './store.js';
import type { CompactionDetails, ListOrder } from './types.js';

export const FUNCTION_IDS = {
  FORK: 'session-tree::fork',
  CLONE: 'session-tree::clone',
  COMPACT: 'session-tree::compact',
  TREE: 'session-tree::tree',
  EXPORT_HTML: 'session-tree::export_html',
  CREATE: 'session-tree::create',
  ENSURE: 'session-tree::ensure',
  APPEND: 'session-tree::append',
  MESSAGES: 'session-tree::messages',
  LIST: 'session-tree::list',
  RECONCILE: 'session-tree::reconcile',
} as const;

export function registerTree(iii: ISdk, store: SessionStore): void {
  iii.registerFunction(
    FUNCTION_IDS.FORK,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const source = requireString(obj, 'source_session_id');
      const from = requireString(obj, 'from_entry_id');
      return { session_id: await fork(store, source, from) };
    },
    { description: 'Fork a session at a given entry into a new session id' },
  );

  iii.registerFunction(
    FUNCTION_IDS.CLONE,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const source = requireString(obj, 'source_session_id');
      return { session_id: await cloneSession(store, source) };
    },
    { description: 'Duplicate a session with re-mapped ids' },
  );

  iii.registerFunction(
    FUNCTION_IDS.COMPACT,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const session_id = requireString(obj, 'session_id');
      const summary = requireString(obj, 'summary');
      const tokens_before = typeof obj.tokens_before === 'number' ? obj.tokens_before : 0;
      const details: CompactionDetails =
        obj.details && typeof obj.details === 'object' ? (obj.details as CompactionDetails) : {};
      const parent_id = typeof obj.parent_id === 'string' ? obj.parent_id : null;
      const entry_id = await compactOp(
        store,
        session_id,
        summary,
        details,
        parent_id,
        tokens_before,
      );
      return { entry_id };
    },
    { description: 'Append a Compaction entry summarising the active path' },
  );

  iii.registerFunction(
    FUNCTION_IDS.TREE,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const session_id = requireString(obj, 'session_id');
      return await tree(store, session_id);
    },
    { description: 'Return the session tree as a nested TreeNode' },
  );

  iii.registerFunction(
    FUNCTION_IDS.EXPORT_HTML,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const session_id = requireString(obj, 'session_id');
      const branch_leaf = typeof obj.branch_leaf === 'string' ? obj.branch_leaf : undefined;
      return { html: await exportHtml(store, session_id, branch_leaf) };
    },
    { description: 'Render the active path as a self-contained HTML document' },
  );

  iii.registerFunction(
    FUNCTION_IDS.CREATE,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const display_name = typeof obj.display_name === 'string' ? obj.display_name : undefined;
      const cwd = typeof obj.cwd === 'string' ? obj.cwd : undefined;
      return { session_id: await createSession(store, display_name, cwd) };
    },
    { description: 'Create a new empty session record' },
  );

  iii.registerFunction(
    FUNCTION_IDS.ENSURE,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const session_id = requireString(obj, 'session_id');
      const display_name = typeof obj.display_name === 'string' ? obj.display_name : undefined;
      const cwd = typeof obj.cwd === 'string' ? obj.cwd : undefined;
      return { session_id: await ensureSession(store, session_id, display_name, cwd) };
    },
    { description: 'Idempotently ensure a session exists with the given id' },
  );

  iii.registerFunction(
    FUNCTION_IDS.APPEND,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const session_id = requireString(obj, 'session_id');
      const parent_id = typeof obj.parent_id === 'string' ? obj.parent_id : null;
      const message = obj.message;
      if (!message) throw new Error('missing required field: message');
      const entry_id = await appendMessage(store, session_id, parent_id, message as AgentMessage);
      return { entry_id };
    },
    { description: 'Append an AgentMessage entry to a session' },
  );

  iii.registerFunction(
    FUNCTION_IDS.MESSAGES,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const session_id = requireString(obj, 'session_id');
      const branch_leaf = typeof obj.branch_leaf === 'string' ? obj.branch_leaf : undefined;
      const messages = await loadMessagesWithEntryIds(store, session_id, branch_leaf);
      return { messages };
    },
    {
      description:
        'Load every AgentMessage on the active path of a session, paired with its entry_id, oldest first',
    },
  );

  iii.registerFunction(
    FUNCTION_IDS.RECONCILE,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const session_id = requireString(obj, 'session_id');
      const snapshot = obj.state_snapshot;
      if (!Array.isArray(snapshot)) throw new Error('missing required field: state_snapshot');
      return await reconcile(store, session_id, snapshot as AgentMessage[]);
    },
    { description: 'Mirror missing messages from a state-snapshot into session-tree' },
  );

  iii.registerFunction(
    FUNCTION_IDS.LIST,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const limit = typeof obj.limit === 'number' ? obj.limit : undefined;
      const offset = typeof obj.offset === 'number' ? obj.offset : undefined;
      const order: ListOrder | undefined =
        obj.order === 'asc' || obj.order === 'desc' ? obj.order : undefined;
      return await listSessions(store, limit, offset, order);
    },
    { description: 'List sessions with optional pagination and ordering' },
  );
}
