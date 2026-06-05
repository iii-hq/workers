/**
 * Shared prune → summarize path for sync and async compaction handlers.
 */

import { logger } from '../runtime/otel.js';
import type { ISdk } from '../runtime/iii.js';
import { emit } from '../turn-orchestrator/events.js';
import { compactionConfig } from './config.js';
import type { ModelLimit } from './overflow.js';
import { prune } from './prune.js';
import {
  type SummarizeOk,
  type SummarizeOptions,
  type SummarizeOutcome,
  summarizeAndAppend,
} from './summarize.js';

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
 * caller has already done the load-bearing work (tree compaction)
 * and the UI marker is a nice-to-have.
 */
async function emitCompactionDone(
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

export type CompactionModel = {
  providerID: string;
  modelID: string;
  modelLimit: ModelLimit;
};

export async function pruneSessionToolOutputs(iii: ISdk, session_id: string): Promise<void> {
  const cfg = compactionConfig();
  await prune(iii, session_id, {
    protectTokens: cfg.pruneProtect,
    minFree: cfg.pruneMinFree,
    protectedTools: cfg.pruneProtectedTools,
  });
}

export async function runSummarizeCompaction(
  iii: ISdk,
  session_id: string,
  options: SummarizeOptions,
  model: CompactionModel,
): Promise<SummarizeOutcome> {
  await pruneSessionToolOutputs(iii, session_id);
  return summarizeAndAppend(iii, session_id, options, {
    providerID: model.providerID,
    modelID: model.modelID,
    modelLimit: model.modelLimit,
  });
}

export async function publishCompactionDone(
  iii: ISdk,
  session_id: string,
  mode: CompactionMode,
  result: SummarizeOk,
): Promise<void> {
  await emitCompactionDone(iii, session_id, mode, {
    summary_text: result.summary_text,
    tokens_before: result.tokens_before,
    compaction_entry_id: result.compaction_entry_id,
    tail_start_id: result.tail_start_id,
  });
}

export function isSummarizeOk(result: SummarizeOutcome): result is SummarizeOk {
  return result !== 'empty' && result.kind === 'ok';
}
