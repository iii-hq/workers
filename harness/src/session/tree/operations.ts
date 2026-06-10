/**
 * Pure session-tree operations. Ports of
 * `session/src/tree/mod.rs::{create_session, append_message, active_path,
 * load_messages, …, fork, clone_session, compact, tree, export_html}`.
 */

import { randomUUID } from 'node:crypto';
import type { AgentContext, AgentMessage } from '../../types/agent-message.js';
import type { SessionStore } from './store.js';
import {
  type CompactionDetails,
  type ListOrder,
  type ListSessionsResult,
  type MessageWithEntryId,
  type SessionEntry,
  SessionError,
  type SessionListRow,
  type SessionMeta,
  type TreeNode,
} from './types.js';

const FORK_SUMMARY_THRESHOLD = 50;

export async function createSession(
  store: SessionStore,
  display_name?: string,
  cwd?: string,
): Promise<string> {
  const session_id = randomUUID();
  const now = Date.now();
  await store.create({
    session_id,
    display_name,
    cwd,
    created_at: now,
    updated_at: now,
    branch_count: 1,
  });
  return session_id;
}

export async function ensureSession(
  store: SessionStore,
  session_id: string,
  display_name?: string,
  cwd?: string,
): Promise<string> {
  try {
    await store.loadMeta(session_id);
    return session_id;
  } catch (err) {
    if (!(err instanceof SessionError) || err.kind !== 'not_found') throw err;
  }
  const now = Date.now();
  await store.create({
    session_id,
    display_name,
    cwd,
    created_at: now,
    updated_at: now,
    branch_count: 0,
  });
  return session_id;
}

export async function appendMessage(
  store: SessionStore,
  session_id: string,
  parent_id: string | null,
  message: AgentMessage,
): Promise<string> {
  let resolvedParent = parent_id;
  if (resolvedParent === null) {
    const path = await activePath(store, session_id);
    resolvedParent = path.at(-1) ?? null;
  }
  const id = randomUUID();
  const entry: SessionEntry = {
    type: 'message',
    id,
    parent_id: resolvedParent,
    message,
    timestamp: Date.now(),
  };
  await store.append(session_id, entry);
  return id;
}

/**
 * Append several messages as one linear chain in a single store round-trip.
 * Equivalent to calling {@link appendMessage} for each message in order, but
 * resolves the active leaf once (not once per message) and writes through
 * {@link SessionStore.appendMany} so meta is refreshed once.
 *
 * Entries share a single append timestamp; their order is carried by the
 * parent chain, not the clock, and {@link activePath} resolves the leaf by
 * chain tip — so a later append with an equal-or-earlier wall-clock timestamp
 * still chains onto this batch's tail instead of being orphaned.
 */
export async function appendMessages(
  store: SessionStore,
  session_id: string,
  parent_id: string | null,
  messages: AgentMessage[],
): Promise<string[]> {
  if (messages.length === 0) return [];

  let parent = parent_id;
  if (parent === null) {
    const path = await activePath(store, session_id);
    parent = path.at(-1) ?? null;
  }

  const timestamp = Date.now();
  const entries: SessionEntry[] = messages.map((message) => {
    const id = randomUUID();
    const entry: SessionEntry = {
      type: 'message',
      id,
      parent_id: parent,
      message,
      timestamp,
    };
    parent = id;
    return entry;
  });

  await store.appendMany(session_id, entries);
  return entries.map((e) => e.id);
}

/**
 * The active leaf is the tip of the newest chain: the most-recent entry that is
 * not the parent of any other entry. `entries` is sorted ascending by
 * (timestamp, id), so we scan from the end and take the first tip. Resolving by
 * chain tip rather than raw sort-max keeps a freshly appended child as the leaf
 * even when its wall-clock timestamp ties or precedes a sibling's — the case a
 * batch append (shared timestamp) or a clock that did not advance would
 * otherwise mis-resolve. Falls back to the most-recent entry only if every
 * entry is some entry's parent (a cycle — should not occur).
 */
function resolveActiveLeaf(entries: SessionEntry[]): string | null {
  const parentIds = new Set<string>();
  for (const e of entries) {
    if (e.parent_id) parentIds.add(e.parent_id);
  }
  for (let i = entries.length - 1; i >= 0; i--) {
    const entry = entries[i];
    if (entry && !parentIds.has(entry.id)) return entry.id;
  }
  return entries.at(-1)?.id ?? null;
}

export async function activePath(
  store: SessionStore,
  session_id: string,
  leaf?: string,
): Promise<string[]> {
  const entries = await store.loadEntries(session_id);
  if (entries.length === 0) return [];
  const byId = new Map(entries.map((e) => [e.id, e] as const));
  const start = leaf ?? resolveActiveLeaf(entries);
  if (start === null) return [];
  const path: string[] = [];
  let cursor: string | null = start;
  while (cursor !== null) {
    path.push(cursor);
    const next: string | null = byId.get(cursor)?.parent_id ?? null;
    cursor = next;
  }
  path.reverse();
  return path;
}

export async function loadMessages(
  store: SessionStore,
  session_id: string,
  leaf?: string,
): Promise<AgentMessage[]> {
  const entries = await store.loadEntries(session_id);
  const path = await activePath(store, session_id, leaf);
  const byId = new Map(entries.map((e) => [e.id, e] as const));
  const out: AgentMessage[] = [];
  for (const id of path) {
    const e = byId.get(id);
    if (e?.type === 'message') out.push(e.message);
  }
  return out;
}

export async function loadMessagesWithEntryIds(
  store: SessionStore,
  session_id: string,
  leaf?: string,
): Promise<MessageWithEntryId[]> {
  const entries = await store.loadEntries(session_id);
  const path = await activePath(store, session_id, leaf);
  const byId = new Map(entries.map((e) => [e.id, e] as const));
  const out: MessageWithEntryId[] = [];
  for (const id of path) {
    const e = byId.get(id);
    if (e?.type === 'message') {
      out.push({ entry_id: id, message: e.message });
    }
  }
  return out;
}

export async function loadContext(
  store: SessionStore,
  session_id: string,
  leaf: string | undefined,
  system_prompt: string,
): Promise<AgentContext> {
  const messages = await loadMessages(store, session_id, leaf);
  return { system_prompt, messages, functions: [] };
}

export async function listSessions(
  store: SessionStore,
  limit?: number,
  offset?: number,
  order: ListOrder = 'desc',
): Promise<ListSessionsResult> {
  const metas = await store.list();
  metas.sort((a, b) =>
    order === 'desc' ? b.updated_at - a.updated_at : a.updated_at - b.updated_at,
  );
  const total = metas.length;
  const page = metas.slice(offset ?? 0, (offset ?? 0) + (limit ?? metas.length));
  const sessions: SessionListRow[] = [];
  for (const meta of page) {
    let entries: SessionEntry[] = [];
    try {
      entries = await store.loadEntries(meta.session_id);
    } catch {}
    sessions.push({
      session_id: meta.session_id,
      created_at: meta.created_at,
      updated_at: meta.updated_at,
      entry_count: entries.length,
      display_name: meta.display_name,
      cwd: meta.cwd,
      last_message_summary: extractSummary(entries),
    });
  }
  return { sessions, total };
}

function extractSummary(entries: SessionEntry[]): string | undefined {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (!e || e.type !== 'message') continue;
    const msg = e.message;
    if (msg.role === 'function_result' || msg.role === 'custom') return undefined;
    for (const block of msg.content) {
      if (block.type === 'text') return block.text.slice(0, 80);
    }
    return undefined;
  }
  return undefined;
}

function withId<T extends SessionEntry>(entry: T, new_id: string): T {
  return { ...entry, id: new_id };
}

function withParent<T extends SessionEntry>(entry: T, new_parent: string | null): T {
  return { ...entry, parent_id: new_parent };
}

export async function fork(
  store: SessionStore,
  source_session_id: string,
  from_entry_id: string,
): Promise<string> {
  const entries = await store.loadEntries(source_session_id);
  const byId = new Map(entries.map((e) => [e.id, e] as const));
  if (!byId.has(from_entry_id)) {
    throw new SessionError('entry_not_found', from_entry_id);
  }
  const path = await activePath(store, source_session_id, from_entry_id);
  const sourceMeta = await store.loadMeta(source_session_id);
  const new_session_id = await createSession(
    store,
    sourceMeta.display_name ? `${sourceMeta.display_name} (fork)` : undefined,
    sourceMeta.cwd,
  );
  if (path.length > FORK_SUMMARY_THRESHOLD) {
    const summary_id = randomUUID();
    await store.append(new_session_id, {
      type: 'branch_summary',
      id: summary_id,
      parent_id: null,
      summary: `Forked from session ${source_session_id} at entry ${from_entry_id}: ${path.length} entries collapsed.`,
      from_id: from_entry_id,
      timestamp: Date.now(),
    });
    return new_session_id;
  }
  const idMap = new Map<string, string>();
  for (const oldId of path) {
    const original = byId.get(oldId);
    if (!original) throw new SessionError('entry_not_found', oldId);
    const new_id = randomUUID();
    const new_parent = original.parent_id ? (idMap.get(original.parent_id) ?? null) : null;
    const copied = withParent(withId(original, new_id), new_parent);
    await store.append(new_session_id, copied);
    idMap.set(oldId, new_id);
  }
  return new_session_id;
}

export async function cloneSession(
  store: SessionStore,
  source_session_id: string,
): Promise<string> {
  const entries = await store.loadEntries(source_session_id);
  const sourceMeta = await store.loadMeta(source_session_id);
  const new_session_id = await createSession(
    store,
    sourceMeta.display_name ? `${sourceMeta.display_name} (clone)` : undefined,
    sourceMeta.cwd,
  );
  const idMap = new Map<string, string>();
  for (const e of entries) idMap.set(e.id, randomUUID());
  for (const e of entries) {
    const new_id = idMap.get(e.id);
    if (!new_id) continue;
    const new_parent = e.parent_id ? (idMap.get(e.parent_id) ?? null) : null;
    await store.append(new_session_id, withParent(withId(e, new_id), new_parent));
  }
  return new_session_id;
}

export async function compact(
  store: SessionStore,
  session_id: string,
  summary: string,
  details: CompactionDetails,
  parent_id: string | null,
  tokens_before: number,
  tail_start_id: string | null = null,
): Promise<string> {
  let resolvedParent = parent_id;
  if (resolvedParent === null) {
    const path = await activePath(store, session_id);
    resolvedParent = path.at(-1) ?? null;
  }
  const id = randomUUID();
  await store.append(session_id, {
    type: 'compaction',
    id,
    parent_id: resolvedParent,
    summary,
    tokens_before,
    tail_start_id,
    details,
    timestamp: Date.now(),
  });
  return id;
}

export type CompactionEntryRow = {
  id: string;
  summary: string;
  tokens_before: number;
  tail_start_id: string | null | undefined;
  details: CompactionDetails;
  timestamp: number;
};

export async function compactionEntries(
  store: SessionStore,
  session_id: string,
): Promise<CompactionEntryRow[]> {
  const entries = await store.loadEntries(session_id);
  return entries
    .filter((e): e is Extract<SessionEntry, { type: 'compaction' }> => e.type === 'compaction')
    .sort((a, b) => a.timestamp - b.timestamp)
    .map((e) => ({
      id: e.id,
      summary: e.summary,
      tokens_before: e.tokens_before,
      tail_start_id: e.tail_start_id,
      details: e.details,
      timestamp: e.timestamp,
    }));
}

export async function appendSynthetic(
  store: SessionStore,
  session_id: string,
  opts: {
    text: string;
    metadata?: unknown;
    parent_id?: string | null;
  },
): Promise<string> {
  const id = randomUUID();
  const parent_id = opts.parent_id ?? null;
  const entry: SessionEntry = {
    type: 'message',
    id,
    parent_id,
    message: {
      role: 'user',
      content: [{ type: 'text', text: opts.text }],
      timestamp: Date.now(),
    },
    timestamp: Date.now(),
  };
  await store.append(session_id, entry);
  return id;
}

export async function updatePart(
  store: SessionStore,
  session_id: string,
  entry_id: string,
  opts: { output: string | null | undefined; compacted_at: unknown },
): Promise<void> {
  await updateParts(store, session_id, [
    { entry_id, output: opts.output, compacted_at: opts.compacted_at },
  ]);
}

export type UpdatePartItem = {
  entry_id: string;
  output: string | null | undefined;
  compacted_at: unknown;
};

export async function updateParts(
  store: SessionStore,
  session_id: string,
  items: UpdatePartItem[],
): Promise<{ updated: number }> {
  if (items.length === 0) return { updated: 0 };
  const entries = await store.loadEntries(session_id);
  const byId = new Map(entries.map((e) => [e.id, e] as const));
  let updated = 0;
  for (const item of items) {
    const target = byId.get(item.entry_id);
    if (!target || target.type !== 'message') continue;
    if (target.message.role !== 'function_result') continue;
    const next: SessionEntry = {
      ...target,
      message: {
        ...target.message,
        content: [{ type: 'text', text: item.output ?? '[output pruned]' }],
        details: {
          ...(target.message.details as Record<string, unknown> | undefined),
          compacted_at: item.compacted_at,
        },
      },
    };
    await store.updateEntry(session_id, item.entry_id, next);
    updated++;
  }
  return { updated };
}

export async function tree(store: SessionStore, session_id: string): Promise<TreeNode> {
  const entries = await store.loadEntries(session_id);
  if (entries.length === 0) {
    throw new SessionError('entry_not_found', `session ${session_id} has no entries`);
  }
  const childrenByParent = new Map<string | null, SessionEntry[]>();
  for (const e of entries) {
    const key = e.parent_id ?? null;
    const list = childrenByParent.get(key) ?? [];
    list.push(e);
    childrenByParent.set(key, list);
  }
  const roots = childrenByParent.get(null) ?? [];
  if (roots.length === 0) {
    throw new SessionError('entry_not_found', `session ${session_id} has no root entry`);
  }
  childrenByParent.delete(null);
  const root = roots[0];
  if (!root) {
    throw new SessionError('entry_not_found', `session ${session_id} has no root entry`);
  }
  return buildNode(root, childrenByParent);
}

function buildNode(entry: SessionEntry, byParent: Map<string | null, SessionEntry[]>): TreeNode {
  const kids = byParent.get(entry.id) ?? [];
  byParent.delete(entry.id);
  return { entry, children: kids.map((c) => buildNode(c, byParent)) };
}

export async function exportHtml(
  store: SessionStore,
  session_id: string,
  branch_leaf?: string,
): Promise<string> {
  const entries = await store.loadEntries(session_id);
  const path = await activePath(store, session_id, branch_leaf);
  const byId = new Map(entries.map((e) => [e.id, e] as const));
  const body = path
    .map((id) => byId.get(id))
    .filter((e): e is SessionEntry => Boolean(e))
    .map((e) => renderEntryHtml(e))
    .join('');
  const title = htmlEscape(session_id);
  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>session ${title}</title>
<style>
  body { background: #0d1117; color: #e6edf3; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", monospace; margin: 0; padding: 24px; }
  .session { max-width: 920px; margin: 0 auto; }
  .entry { padding: 12px 16px; margin: 12px 0; border-radius: 6px; border-left: 3px solid #30363d; white-space: pre-wrap; word-wrap: break-word; }
  .role { font-weight: 600; font-size: 0.78em; letter-spacing: 0.08em; text-transform: uppercase; opacity: 0.7; margin-bottom: 6px; }
  .user { background: #0b2730; border-left-color: #06b6d4; color: #67e8f9; }
  .assistant { background: #161b22; border-left-color: #c9d1d9; color: #f0f6fc; }
  .tool-result { background: #161b22; border-left-color: #6e7681; color: #8b949e; opacity: 0.85; }
  .thinking { font-style: italic; opacity: 0.75; border-left-color: #a371f7; }
  .custom { background: #161b22; border-left-color: #d29922; }
  .summary { background: #1c1d23; border-left-color: #f0883e; }
  .compaction { background: #1c1d23; border-left-color: #2ea043; }
  pre { margin: 0; font-family: inherit; }
</style>
</head>
<body>
<div class="session">
${body}</div>
</body>
</html>`;
}

function renderEntryHtml(entry: SessionEntry): string {
  if (entry.type === 'message') return renderMessageHtml(entry.message);
  if (entry.type === 'custom_message') {
    const text = entry.display ?? JSON.stringify(entry.content);
    return `<div class="entry custom"><div class="role">custom · ${htmlEscape(entry.custom_type)}</div><pre>${htmlEscape(text)}</pre></div>\n`;
  }
  if (entry.type === 'branch_summary') {
    return `<div class="entry summary"><div class="role">branch summary · from ${htmlEscape(entry.from_id)}</div><pre>${htmlEscape(entry.summary)}</pre></div>\n`;
  }
  const read = (entry.details.read_files ?? []).map(htmlEscape).join(', ');
  const modified = (entry.details.modified_files ?? []).map(htmlEscape).join(', ');
  return `<div class="entry compaction"><div class="role">compaction · ${entry.tokens_before} tokens before</div><pre>${htmlEscape(entry.summary)}\n\nread: ${read}\nmodified: ${modified}</pre></div>\n`;
}

function renderMessageHtml(msg: AgentMessage): string {
  if (msg.role === 'user')
    return `<div class="entry user"><div class="role">user</div>${renderBlocksHtml(msg.content)}</div>\n`;
  if (msg.role === 'assistant')
    return `<div class="entry assistant"><div class="role">assistant · ${htmlEscape(msg.model)}</div>${renderBlocksHtml(msg.content)}</div>\n`;
  if (msg.role === 'function_result')
    return `<div class="entry tool-result"><div class="role">function result · ${htmlEscape(msg.function_id)}</div>${renderBlocksHtml(msg.content)}</div>\n`;
  const text = msg.display ?? JSON.stringify(msg.content);
  return `<div class="entry custom"><div class="role">custom · ${htmlEscape(msg.custom_type)}</div><pre>${htmlEscape(text)}</pre></div>\n`;
}

function renderBlocksHtml(blocks: import('../../types/content.js').ContentBlock[]): string {
  let out = '';
  for (const b of blocks) {
    switch (b.type) {
      case 'text':
        out += `<pre>${htmlEscape(b.text)}</pre>`;
        break;
      case 'thinking':
        out += `<div class="thinking"><pre>${htmlEscape(b.text)}</pre></div>`;
        break;
      case 'image':
        out += `<pre>[image: ${htmlEscape(b.mime)}]</pre>`;
        break;
      case 'function_call':
        out += `<pre>function call: ${htmlEscape(b.function_id)} ${htmlEscape(JSON.stringify(b.arguments))}</pre>`;
        break;
      case 'function_result':
        out += renderBlocksHtml(b.content);
        break;
    }
  }
  return out;
}

function htmlEscape(s: string): string {
  return s
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

export type { SessionMeta };
