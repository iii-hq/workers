/**
 * `context-compaction::compact_session` — the thin user-initiated `/compact`
 * wrapper kept for the console's existing RPC contract.
 *
 * All real context logic now lives in the standalone Rust `context-manager`
 * worker (`context::*`). This handler:
 *   1. loads the post-compaction window + anchored summary from session-manager,
 *   2. calls `context::compact` (storage-agnostic — it only returns a summary),
 *   3. persists the compaction round trip as an additive `kind:"custom"`
 *      bookkeeping entry (the transcript itself is never modified),
 * and maps the `context::compact` union back onto the console's `CompactResult`
 * wire shape (`status` / `tail_start_id` / `tokens_before` / `summary_text` /
 * `auto_continued`).
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import {
  type AppliedReport,
  compactWindow,
  loadAssembleWindow,
  modelInputFromConsole,
  persistCompactionRoundTrip,
} from '../runtime/compaction.js';

type CompactSessionResult =
  | {
      status: 'ok';
      tail_start_id: string | null;
      tokens_before: number;
      auto_continued: boolean;
      summary_text: string;
    }
  | { status: 'busy' }
  | { status: 'empty' }
  | { status: 'overflow'; message: string }
  | { status: 'error'; message: string };

async function compactSession(iii: ISdk, payload: unknown): Promise<CompactSessionResult> {
  const obj = (payload ?? {}) as Record<string, unknown>;
  const session_id = typeof obj.session_id === 'string' ? obj.session_id : '';
  if (!session_id) {
    throw new Error('context-compaction::compact_session: session_id is required');
  }

  const model = modelInputFromConsole(obj.model);
  if (!model) {
    return { status: 'error', message: 'model { id, providerID } is required' };
  }

  try {
    const { window, previousSummary } = await loadAssembleWindow(iii, session_id);
    const result = await compactWindow(iii, session_id, window, model, previousSummary);

    if (result.status === 'busy') return { status: 'busy' };
    if (result.status === 'empty') return { status: 'empty' };
    if (result.status === 'overflow') {
      return {
        status: 'overflow',
        message: 'compaction unavailable (no llm-router or the summariser failed)',
      };
    }

    // ok — persist the round trip so the next turn anchors on this summary.
    const applied: AppliedReport = {
      pruned: false,
      pruned_tokens: 0,
      compacted: true,
      summary: result.summary,
      tail_start_index: result.tail_start_index,
      tokens_before: result.tokens_before,
    };
    await persistCompactionRoundTrip(iii, session_id, applied, window);

    const idx = result.tail_start_index;
    const tailEntry =
      typeof idx === 'number' && idx >= 0 && idx < window.length ? window[idx] : undefined;
    return {
      status: 'ok',
      tail_start_id: tailEntry ? tailEntry.entry_id : null,
      tokens_before: result.tokens_before,
      // /compact runs against a conversation at rest; no synthetic continue.
      auto_continued: false,
      summary_text: result.summary,
    };
  } catch (err) {
    logger.warn('compact_session failed', { session_id, err: String(err) });
    return { status: 'error', message: err instanceof Error ? err.message : String(err) };
  }
}

export async function register(iii: ISdk): Promise<void> {
  iii.registerFunction(
    'context-compaction::compact_session',
    async (payload: unknown) => compactSession(iii, payload),
    {
      description:
        'User-initiated synchronous compaction of a session. Required: session_id. Optional: model { id, providerID, limit? }. Summarises the post-compaction window via context::compact and persists the compaction round trip as a custom bookkeeping entry; the transcript is left untouched.',
    },
  );
}
