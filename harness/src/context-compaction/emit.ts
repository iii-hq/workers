/**
 * Shared helper for emitting `compaction_done` after sync/async
 * handlers finish rewriting flat-state. Pre-extraction this exact
 * try/catch + payload block lived byte-for-byte in both handlers; the
 * helper keeps the two handlers in sync and gives the failure log a
 * stable code for monitoring.
 */
import { logger } from '../runtime/otel.js';
import type { ISdk } from '../runtime/iii.js';
import { emit } from '../turn-orchestrator/events.js';

export type CompactionMode = 'sync' | 'async';

export interface CompactionDonePayload {
  summary_text: string;
  tokens_before: number;
  compaction_entry_id: string;
  /** First entry_id of the preserved tail; null when nothing was kept. */
  tail_start_id: string | null;
}

/**
 * Best-effort: a publish failure is logged but never thrown — the
 * caller has already done the load-bearing work (rewriting flat
 * state) and the UI marker is a nice-to-have.
 */
export async function emitCompactionDone(
  iii: ISdk,
  session_id: string,
  mode: CompactionMode,
  payload: CompactionDonePayload,
): Promise<void> {
  try {
    await emit(iii, session_id, {
      type: 'compaction_done',
      mode,
      summary_text: payload.summary_text,
      tokens_before: payload.tokens_before,
      compaction_entry_id: payload.compaction_entry_id,
      tail_start_id: payload.tail_start_id,
    });
  } catch (err) {
    logger.warn(`handler-${mode}: compaction_done emit failed`, {
      code: 'compaction_done_emit_failed',
      session_id,
      err: String(err),
    });
  }
}
