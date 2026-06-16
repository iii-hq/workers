/**
 * `session::*` client for the session-manager worker — the durable,
 * reactive conversation store this harness drives (see
 * `session-manager/architecture/integration.md`).
 *
 * Conventions encoded here:
 * - Every read of the active path MUST paginate (`session::messages`
 *   defaults to 50 rows, hard cap 500 per page); `readActivePath` owns the
 *   cursor loop so call sites can assume a full read.
 * - Append idempotency rides on deterministic `entry_id`s derived from
 *   durable inputs (message_id, turn_count, function_call_id). Replays
 *   return the original entry and fire nothing.
 * - Compaction records are `kind:"custom"` entries (`custom_type:
 *   "compaction"`); path order is their authoritative order.
 */

import type { ISdk } from './iii.js';
import { logger } from './otel.js';
import type { AgentMessage } from '../types/agent-message.js';
import type { ContentBlock } from '../types/content.js';

export type SessionStatus = 'idle' | 'working' | 'done' | 'error';

/** One row of `session::messages` — exactly one of `message` / `custom`. */
export type TranscriptItem = {
  entry_id: string;
  message?: AgentMessage;
  custom?: { custom_type: string; data: unknown };
};

export type MessageWithEntryId = {
  entry_id: string;
  message: AgentMessage;
};

export type AppendResult = {
  entry_id: string;
  parent_id: string | null;
  timestamp: number;
};

export const COMPACTION_CUSTOM_TYPE = 'compaction';

/** Payload stored in a compaction custom entry's `data`. */
export type CompactionData = {
  summary: string;
  tokens_before: number;
  /**
   * Entry id where the verbatim tail begins (spec name, harness.md
   * § Compaction persistence). `null` when the whole head was summarised.
   * Read back with a legacy `tail_start_id` fallback for pre-migration
   * sessions (see {@link compactionRowsFromPath}).
   */
  tail_start_entry_id: string | null;
  details: Record<string, unknown>;
  /** Wall-clock at write time; path order remains the authoritative order. */
  timestamp: number;
};

export type CompactionRow = CompactionData & { entry_id: string };

const DEFAULT_TIMEOUT_MS = 10_000;
/** `session::messages` hard cap per page. */
const MESSAGES_PAGE_LIMIT = 500;

/** Match a session-manager error by stable code (`session/<snake>: …`). */
export function isSessionErrorCode(err: unknown, code: string): boolean {
  return String(err).includes(`session/${code}`);
}

// ── Deterministic entry ids (the idempotency contract) ─────────────────────

/** Run-unique key: the console's per-send message_id, else a per-run stamp. */
export function runKey(rec: {
  message_id?: string;
  session_id: string;
  started_at_ms: number;
}): string {
  return rec.message_id ?? `${rec.session_id}-${rec.started_at_ms}`;
}

export function userEntryId(messageId: string, index: number): string {
  return `${messageId}-user-${index}`;
}

export function assistantEntryId(runKey: string, turn: number): string {
  return `${runKey}-t${turn}-assistant`;
}

export function functionResultEntryId(functionCallId: string): string {
  return `fr-${functionCallId}`;
}

// ── Lifecycle ───────────────────────────────────────────────────────────────

export async function sessionEnsure(
  iii: ISdk,
  session_id: string,
  opts?: { title?: string; description?: string; metadata?: Record<string, unknown> },
): Promise<{ created: boolean }> {
  const resp = await iii.trigger<unknown, { session_id: string; created: boolean }>({
    function_id: 'session::ensure',
    payload: { session_id, ...opts },
    timeoutMs: DEFAULT_TIMEOUT_MS,
  });
  return { created: resp?.created === true };
}

/**
 * Best-effort status flip. Status is a UX signal — a failed set must never
 * abort a turn, so failures are logged and swallowed. Same-status calls are
 * server-side no-ops; `reason` is stored only with `"error"`.
 */
export async function sessionSetStatus(
  iii: ISdk,
  session_id: string,
  status: SessionStatus,
  reason?: string,
): Promise<void> {
  try {
    await iii.trigger({
      function_id: 'session::set-status',
      payload: { session_id, status, ...(reason ? { reason } : {}) },
      timeoutMs: DEFAULT_TIMEOUT_MS,
    });
  } catch (err) {
    logger.warn('session::set-status failed', { session_id, status, err: String(err) });
  }
}

// ── Messages ────────────────────────────────────────────────────────────────

export async function sessionAppendMessage(
  iii: ISdk,
  input: {
    session_id: string;
    message: AgentMessage;
    entry_id?: string;
    parent_id?: string;
    origin?: Record<string, unknown>;
  },
): Promise<AppendResult> {
  const resp = await iii.trigger<unknown, AppendResult>({
    function_id: 'session::append',
    payload: input,
    timeoutMs: DEFAULT_TIMEOUT_MS,
  });
  if (!resp || typeof resp.entry_id !== 'string') {
    throw new Error('session::append returned no entry_id');
  }
  return resp;
}

export async function sessionAppendCustom(
  iii: ISdk,
  input: {
    session_id: string;
    custom_type: string;
    data: unknown;
    entry_id?: string;
    parent_id?: string;
    origin?: Record<string, unknown>;
  },
): Promise<AppendResult> {
  const { session_id, custom_type, data, ...rest } = input;
  const resp = await iii.trigger<unknown, AppendResult>({
    function_id: 'session::append',
    payload: { session_id, custom: { custom_type, data }, ...rest },
    timeoutMs: DEFAULT_TIMEOUT_MS,
  });
  if (!resp || typeof resp.entry_id !== 'string') {
    throw new Error('session::append returned no entry_id');
  }
  return resp;
}

export async function sessionUpdateMessage(
  iii: ISdk,
  input: {
    session_id: string;
    entry_id: string;
    content: ContentBlock[];
    details?: unknown;
    expected_revision?: number;
    origin?: Record<string, unknown>;
  },
): Promise<{ updated: boolean; revision: number }> {
  const resp = await iii.trigger<unknown, { updated: boolean; revision: number }>({
    function_id: 'session::update-message',
    payload: input,
    timeoutMs: DEFAULT_TIMEOUT_MS,
  });
  return resp ?? { updated: false, revision: 0 };
}

/**
 * Full active path, oldest first, looping `cursor` until exhausted.
 * `include_custom` interleaves `kind:"custom"` entries at their path
 * position; `roles` narrows to matching message roles.
 */
export async function readActivePath(
  iii: ISdk,
  session_id: string,
  opts?: { include_custom?: boolean; roles?: string[] },
): Promise<TranscriptItem[]> {
  const items: TranscriptItem[] = [];
  let cursor: string | undefined;
  for (;;) {
    const resp = await iii.trigger<
      unknown,
      { messages?: TranscriptItem[]; next_cursor?: string | null }
    >({
      function_id: 'session::messages',
      payload: {
        session_id,
        limit: MESSAGES_PAGE_LIMIT,
        ...(cursor ? { cursor } : {}),
        ...(opts?.include_custom ? { include_custom: true } : {}),
        ...(opts?.roles ? { roles: opts.roles } : {}),
      },
      timeoutMs: DEFAULT_TIMEOUT_MS,
    });
    const page = resp?.messages ?? [];
    for (const item of page) {
      if (item && typeof item.entry_id === 'string') items.push(item);
    }
    const next = resp?.next_cursor;
    if (!next || page.length === 0) return items;
    cursor = next;
  }
}

// ── Path helpers ────────────────────────────────────────────────────────────

/** `kind:"message"` rows of a path read, with entry ids. */
export function messageItemsFromPath(items: TranscriptItem[]): MessageWithEntryId[] {
  const out: MessageWithEntryId[] = [];
  for (const item of items) {
    if (item.message) out.push({ entry_id: item.entry_id, message: item.message });
  }
  return out;
}

/**
 * Compaction records on the path, in path order (== chronological: each
 * record is appended at the then-current leaf).
 */
export function compactionRowsFromPath(items: TranscriptItem[]): CompactionRow[] {
  const rows: CompactionRow[] = [];
  for (const item of items) {
    if (item.custom?.custom_type !== COMPACTION_CUSTOM_TYPE) continue;
    // Legacy `tail_start_id` is read for sessions written before the
    // rename to the spec's `tail_start_entry_id`.
    const data = (item.custom.data ?? {}) as Partial<CompactionData> & {
      tail_start_id?: unknown;
    };
    if (typeof data.summary !== 'string') continue;
    const tailNew = typeof data.tail_start_entry_id === 'string' ? data.tail_start_entry_id : null;
    const tailLegacy = typeof data.tail_start_id === 'string' ? data.tail_start_id : null;
    rows.push({
      entry_id: item.entry_id,
      summary: data.summary,
      tokens_before: typeof data.tokens_before === 'number' ? data.tokens_before : 0,
      tail_start_entry_id: tailNew ?? tailLegacy,
      details:
        data.details && typeof data.details === 'object'
          ? (data.details as Record<string, unknown>)
          : {},
      timestamp: typeof data.timestamp === 'number' ? data.timestamp : 0,
    });
  }
  return rows;
}
